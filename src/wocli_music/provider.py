"""Kuwo transport extracted and adapted from ../music/kw_full.py.

No global urllib opener, disabled TLS verification, or account state is installed.
"""

import ast
import base64
import hashlib
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zlib
from pathlib import Path

from .lyrics import lines_to_lrc, lyric_matches, lyric_title, parse_lrc
from .models import (
    AudioUnavailable,
    IncompleteAudio,
    MusicError,
    NetworkError,
    Stream,
    Track,
    clean,
    normalize_id,
)

VERSION = "MUSIC_8.7.4.0_BDS3"
MODULE = "kwmusic_web_1_bds_20171222.exe"
CLIENT_ID = "58365624"
USER_AGENT = "Mozilla/4.0 (compatible; MSIE 6.0; Windows NT 5.0; .NET CLR 1.1.4322)"
QUALITY_MAP = [("flac", "ALFLAC", ".flac"), ("320k", "MP3H", ".mp3"), ("128k", "MP3128", ".mp3")]


def _xor(data: bytes, key: bytes) -> bytes:
    return bytes(byte ^ key[i % len(key)] for i, byte in enumerate(data))


def _right(data: bytes, amount: int) -> bytes:
    amount %= len(data)
    return data[-amount:] + data[:-amount] if amount else data


def _left(data: bytes, amount: int) -> bytes:
    amount %= len(data)
    return data[amount:] + data[:amount]


def encode_query(text: str) -> bytes:
    data = _xor(_right(text.encode(), 0x85), b"KoOtOiTvINGwd")
    return base64.b64encode(_xor(_left(data, 0x47), b"_Y8g2E6n0E1i7L5t2IoOoNk"))


def decode_reply(data: bytes) -> bytes:
    try:
        data = base64.b64decode(data, validate=True)
    except ValueError:
        pass
    if not data:
        raise MusicError("音源服务返回了空响应")
    data = _right(_xor(data, b"_Y8g2E6n0E1i7L5t2IoOoNk"), 0x47)
    data = _left(_xor(data, b"KoOtOiTvINGwd"), 0x85)
    try:
        return zlib.decompress(data)
    except zlib.error:
        return data


class KuwoProvider:
    def __init__(self, *, direct=False, timeout=10):
        self.timeout = timeout
        self.opener = (
            urllib.request.build_opener(urllib.request.ProxyHandler({}))
            if direct
            else urllib.request.build_opener()
        )

    def _open(self, url, *, data=None, headers=None, timeout=None):
        if urllib.parse.urlsplit(url).scheme not in ("http", "https"):
            raise MusicError("音源返回了无效的媒体地址")
        request = urllib.request.Request(
            url, data=data, headers={"User-Agent": USER_AGENT, **(headers or {})}
        )
        try:
            return self.opener.open(request, timeout=timeout or self.timeout)
        except (urllib.error.URLError, OSError, TimeoutError) as error:
            # Avoid showing signed CDN URLs or proxy credentials in errors.
            if isinstance(error, urllib.error.HTTPError):
                detail = f"HTTP {error.code}"
            else:
                detail = type(error).__name__
            raise NetworkError(f"音源请求失败（{detail}），请检查网络或使用 --direct") from error

    def _read(self, url, **kwargs) -> bytes:
        with self._open(url, **kwargs) as response:
            return response.read(4 * 1024 * 1024)

    def _json(self, url, **kwargs):
        try:
            raw = self._read(url, **kwargs).decode("utf8", "replace")
            try:
                return json.loads(raw)
            except ValueError:
                return ast.literal_eval(raw)
        except (ValueError, UnicodeError, SyntaxError) as error:
            raise MusicError("音源服务响应格式已变化") from error

    @staticmethod
    def _track(item: dict) -> Track:
        return Track(
            normalize_id(str(item.get("MUSICRID") or item.get("RID") or "")),
            clean(item.get("SONGNAME") or item.get("NAME")) or "未知曲目",
            clean(item.get("ARTIST")) or "未知歌手",
            clean(item.get("ALBUM")),
            float(item.get("DURATION") or 0),
            str(item.get("FORMATS") or ""),
        )

    def search(self, keyword: str, page=0, limit=20) -> tuple[int, list[Track]]:
        params = urllib.parse.urlencode(
            {
                "all": keyword,
                "ft": "music",
                "newsearch": 1,
                "alflac": 1,
                "itemset": "web_2013",
                "client": "kt",
                "cluster": 0,
                "pn": page,
                "rn": limit,
                "vermerge": 1,
                "rformat": "json",
                "encoding": "utf8",
                "show_copyright_off": 1,
                "pcmp4": 1,
                "vipver": VERSION,
                "plat": "pc",
                "devid": CLIENT_ID,
            }
        )
        raw = self._read(f"https://search.kuwo.cn/r.s?{params}").decode("utf-8", "replace")
        try:
            try:
                result = json.loads(raw)
            except ValueError:
                result = ast.literal_eval(raw)
            tracks = [self._track(item) for item in result.get("abslist", [])]
            return int(result.get("TOTAL", 0)), tracks
        except (ValueError, TypeError, KeyError, SyntaxError, AttributeError) as error:
            raise MusicError("无法读取搜索结果，音源接口可能已变化") from error

    def details(self, identity: str) -> Track:
        rid = normalize_id(identity).split(":")[1]
        params = urllib.parse.urlencode(
            {"type": "music", "rformat": "json", "encoding": "utf8", "rid": f"MUSIC_{rid}"}
        )
        result = self._json(f"https://search.kuwo.cn/r.s?{params}")
        items = result if isinstance(result, list) else result.get("abslist", [])
        for item in items:
            # This endpoint uses lower-case fields on some server versions.
            mapped = {key.upper(): value for key, value in item.items()}
            mapped.setdefault("MUSICRID", f"MUSIC_{rid}")
            track = self._track(mapped)
            if track.id == normalize_id(identity):
                return track
        raise MusicError("无法获取该歌曲信息，请先搜索后使用结果中的 ID")

    def qualities(self, track: Track) -> list[dict]:
        rid = normalize_id(track.id).split(":")[1]
        raw = self._read(
            "http://search.kuwo.cn/r.s?stype=musicinfo&itemset=music_2014"
            f"&alflac=1&pcmp4=1&ids=MUSIC_{rid}"
        )
        try:
            if len(raw) < 8:
                raise ValueError("short header")
            length = int.from_bytes(raw[4:8], "little")
            text = zlib.decompress(raw[8 : 8 + length]).decode("utf8", "replace")
        except (zlib.error, ValueError) as error:
            raise MusicError("无法解析歌曲音质信息") from error
        info = dict(line.split("=", 1) for line in text.splitlines() if "=" in line)
        qualities = []
        for quality, key, extension in QUALITY_MAP:
            match = re.match(r"S1:(\d+)\|S2:(\d+)\|SIZE:(\d+)", info.get(key, ""))
            if match:
                qualities.append(
                    {
                        "quality": quality,
                        "s1": int(match[1]),
                        "s2": int(match[2]),
                        "size": int(match[3]),
                        "extension": extension,
                    }
                )
        return qualities

    def _resolve(self, quality: dict) -> tuple[str, int]:
        query = (
            f"<001><U_QRY>|<{quality['s1']},{quality['s2']}>|"
            f"<{CLIENT_ID}><{VERSION}><{MODULE}>|<>|<rid>|"
            "<uip:0.0.0.0>|<new>|<nat:3>|<flags:262144><speer>|<ipdeny:no>|<loginid:>"
        )
        raw = self._read(
            "http://deliver.kuwo.cn/yl_res_manage.search",
            data=encode_query(query),
            headers={"Connection": "close"},
        )
        text = decode_reply(raw).decode("utf8", "replace")
        url = re.search(r"<URL><1>\|<([^>]+)>", text)
        size = re.search(r"<FILE_LEN><(\d+)>", text)
        if not url:
            raise AudioUnavailable("该档位未提供播放地址")
        return url[1], int(size[1]) if size else quality["size"]

    def _public_url(self, track: Track, quality: dict) -> str:
        params = urllib.parse.urlencode(
            {
                "type": "convert_url3",
                "rid": normalize_id(track.id).split(":")[1],
                "format": "mp3",
                "br": quality["quality"],
                "response": "url",
            }
        )
        result = self._json(f"https://antiserver.kuwo.cn/anti.s?{params}")
        if (
            not isinstance(result, dict)
            or not isinstance(result.get("url"), str)
            or not result["url"]
        ):
            raise AudioUnavailable("公开播放接口未提供地址")
        return result["url"]

    def _probe(self, url: str, expected_size: int):
        with self._open(url, headers={"Range": "bytes=0-4095"}) as response:
            head = response.read(4096)
            valid = head.startswith((b"ID3", b"fLaC", b"OggS", b"RIFF")) or (
                len(head) > 1 and head[0] == 255 and head[1] & 0xE0 == 0xE0
            )
            if not valid:
                raise MusicError("返回内容不是可识别的音频")
            content_range = response.headers.get("Content-Range", "")
            total = content_range.rsplit("/", 1)[-1] if "/" in content_range else ""
            if not total.isdigit():
                total = response.headers.get("Content-Length", "")
            if total.isdigit() and expected_size and int(total) < expected_size * 0.8:
                raise IncompleteAudio("上游返回的音频明显短于预期，可能只有试听片段")
            return int(total) if total.isdigit() else 0

    def stream(self, track: Track, preference="320k") -> Stream:
        if track.path:
            if not Path(track.path).is_file():
                raise MusicError("本地音频已移动或删除")
            return Stream(
                track.path, "local", Path(track.path).stat().st_size, Path(track.path).suffix
            )
        qualities = self.qualities(track)
        names = [quality for quality, _, _ in QUALITY_MAP]
        if preference != "best":
            rank = names.index(preference)
            qualities = [q for q in qualities if names.index(q["quality"]) >= rank]
        failures = []
        errors = []
        for quality in qualities:
            try:
                url, size = self._resolve(quality)
                self._probe(url, quality["size"])
                return Stream(url, quality["quality"], size, quality["extension"])
            except MusicError as error:
                failures.append(f"P2P {quality['quality']}: {error}")
                errors.append(error)
        # Some newer tracks have no legacy P2P resource. Resolve the same ID using
        # the public playback endpoint, then validate it just as strictly.
        for quality in qualities:
            if quality["extension"] != ".mp3":
                continue
            try:
                url = self._public_url(track, quality)
                size = self._probe(url, quality["size"])
                return Stream(url, quality["quality"], size or quality["size"], ".mp3")
            except MusicError as error:
                failures.append(f"公开接口 {quality['quality']}: {error}")
                errors.append(error)
        if any(isinstance(error, IncompleteAudio) for error in errors):
            raise IncompleteAudio(
                "该版本仅返回试听或不完整音频，未获取到可播放的完整版。", failures
            )
        if not errors or all(isinstance(error, AudioUnavailable) for error in errors):
            raise AudioUnavailable("当前音源未提供这首歌的播放地址，请选择其他歌曲。", failures)
        detail = "；".join(failures)
        if errors and all(isinstance(error, NetworkError) for error in errors):
            raise NetworkError("音源网络请求失败，请稍后重试")
        raise MusicError(f"该歌曲当前无法播放。{detail}")

    def lyrics(self, track: Track) -> str:
        if track.path:
            sidecar = Path(track.path).with_suffix(".lrc")
            return sidecar.read_text(encoding="utf-8-sig") if sidecar.exists() else ""
        deadline = time.monotonic() + 20

        def query(url, **kwargs):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("歌词获取超时")
            result = self._json(url, timeout=min(4, remaining), **kwargs)
            if not isinstance(result, dict):
                raise ValueError("Invalid lyric response")
            return result

        recoverable = (MusicError, OSError, ValueError, TypeError, KeyError, AttributeError)
        # Prefer native lyrics for this exact Kuwo ID. The H5 service requires
        # its normal mobile Referer/User-Agent headers, even though it has no login.
        rid = normalize_id(track.id).split(":")[1]
        for scheme in ("https", "http"):
            try:
                data = query(
                    f"{scheme}://m.kuwo.cn/newh5/singles/songinfoandlrc?musicId={rid}&httpsStatus=1",
                    headers={
                        "Referer": "https://m.kuwo.cn/h5app/",
                        "User-Agent": "Mozilla/5.0 (Linux; Android 12) AppleWebKit/537.36 Mobile Safari/537.36",
                    },
                )
                text = lines_to_lrc((data.get("data") or {}).get("lrclist") or [])
                if parse_lrc(text):
                    return text
            except recoverable:
                continue

        title = lyric_title(track.title)
        keywords = list(dict.fromkeys((f"{track.artist} {title}", title)))
        for keyword in keywords:
            try:
                params = urllib.parse.urlencode(
                    {
                        "ver": 1,
                        "man": "yes",
                        "client": "mobi",
                        "keyword": keyword,
                        "duration": int(track.duration * 1000),
                        "hash": "",
                    }
                )
                data = query(f"https://krcs.kugou.com/search?{params}")
                for candidate in (data.get("candidates") or [])[:10]:
                    try:
                        artists = re.split(r"[&/、,，]", clean(candidate.get("singer")))
                        duration = float(candidate.get("duration") or 0) / 1000
                        if not lyric_matches(
                            track, clean(candidate.get("song")), artists, duration
                        ):
                            continue
                        params = urllib.parse.urlencode(
                            {
                                "ver": 1,
                                "client": "pc",
                                "id": candidate["id"],
                                "accesskey": candidate["accesskey"],
                                "fmt": "lrc",
                                "charset": "utf8",
                            }
                        )
                        result = query(f"https://lyrics.kugou.com/download?{params}")
                        text = base64.b64decode(result.get("content") or "").decode(
                            "utf8", "replace"
                        )
                        if parse_lrc(text):
                            return text
                    except recoverable:
                        continue
            except recoverable:
                continue

        for keyword in keywords:
            try:
                params = urllib.parse.urlencode({"type": 1, "limit": 10, "s": keyword})
                data = query(f"https://music.163.com/api/search/get/web?{params}")
                for song in (data.get("result") or {}).get("songs") or []:
                    try:
                        artists = [artist.get("name", "") for artist in song.get("artists", [])]
                        duration = float(song.get("duration") or song.get("dt") or 0) / 1000
                        if not lyric_matches(track, clean(song.get("name")), artists, duration):
                            continue
                        result = query(
                            "https://music.163.com/api/song/lyric?os=pc&lv=-1&kv=-1&tv=-1"
                            f"&id={song['id']}"
                        )
                        text = (result.get("lrc") or {}).get("lyric") or ""
                        if parse_lrc(text):
                            return text
                    except recoverable:
                        continue
            except recoverable:
                continue
        return ""

    def download(
        self, stream: Stream, target: Path, *, overwrite=False, progress=None, cancelled=None
    ):
        if target.exists() and not overwrite:
            raise MusicError(f"文件已存在：{target}（使用 --force 覆盖）")
        target.parent.mkdir(parents=True, exist_ok=True)
        temporary = target.with_name(f".{target.name}.{uuid.uuid4().hex[:8]}.part")
        digest = hashlib.md5()
        received = 0
        expected = stream.size
        server_md5 = None
        try:
            with temporary.open("xb") as output:
                while True:
                    if cancelled and cancelled.is_set():
                        raise MusicError("下载已取消")
                    with self._open(
                        stream.url, headers={"Range": f"bytes={received}-{received + 1048575}"}
                    ) as response:
                        range_header = response.headers.get("Content-Range", "")
                        if response.status == 206:
                            match = re.fullmatch(r"bytes (\d+)-(\d+)/(\d+)", range_header)
                            if not match or int(match[1]) != received:
                                raise MusicError("服务器返回了错误的音频分块")
                            expected = int(match[3])
                        elif response.status != 200 or received:
                            raise MusicError("服务器不支持续传")
                        server_md5 = response.headers.get("X-COS-META-MD5") or server_md5
                        before = received
                        while block := response.read(65536):
                            if cancelled and cancelled.is_set():
                                raise MusicError("下载已取消")
                            output.write(block)
                            digest.update(block)
                            received += len(block)
                            if progress:
                                progress(received, expected)
                        if received == before:
                            raise MusicError("音频下载意外中断")
                    if received >= expected:
                        break
            if cancelled and cancelled.is_set():
                raise MusicError("下载已取消")
            if received != expected:
                raise MusicError("音频长度校验失败")
            if server_md5 and digest.hexdigest().lower() != server_md5.lower():
                raise MusicError("音频 MD5 校验失败")
            temporary.replace(target)
        finally:
            temporary.unlink(missing_ok=True)
