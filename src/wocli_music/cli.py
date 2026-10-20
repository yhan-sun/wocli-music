import argparse
import asyncio
import json
import re
import sys
import threading
from pathlib import Path

from rich.console import Console
from rich.progress import Progress
from rich.prompt import IntPrompt
from rich.table import Table

from . import __version__
from .client import Client
from .config import MODES, QUALITIES, Paths, Settings, mpv_command
from .models import MusicError, Track, clock, normalize_id
from .provider import KuwoProvider
from .store import Store

console = Console(highlight=False)
errors = Console(stderr=True, highlight=False)


def common(parser):
    parser.add_argument(
        "--json", action="store_true", default=argparse.SUPPRESS, help="输出 JSON，供脚本调用"
    )
    parser.add_argument(
        "--direct",
        action="store_true",
        default=argparse.SUPPRESS,
        help="音源请求忽略系统代理，使用直连",
    )
    return parser


def song_arguments(parser):
    parser.add_argument("keyword", nargs="*", help="歌名、歌手或本地音频路径")
    parser.add_argument("--id", help="歌曲 ID，例如 kuwo:228908")
    parser.add_argument("--pick", type=int, help="选择搜索结果中的第 N 首（从 1 开始）")


def make_parser():
    parser = common(argparse.ArgumentParser(prog="wocli", description="无需登录的终端音乐播放器"))
    parser.set_defaults(json=False, direct=False)
    parser.add_argument("--version", action="version", version=f"wocli-music {__version__}")
    commands = parser.add_subparsers(dest="command")

    def command(name, help):
        return common(commands.add_parser(name, help=help))

    command("tui", "打开终端音乐工作台")
    search = command("search", "搜索酷我曲库")
    search.add_argument("keyword", nargs="+")
    search.add_argument("--page", type=int, default=1)
    search.add_argument("--limit", type=int, default=20)
    play = command("play", "播放歌曲或恢复上次队列")
    song_arguments(play)
    play.add_argument("--quality", choices=QUALITIES)
    play.add_argument("--no-wait", action="store_true", help="提交播放请求后立即返回")
    for name, help in [
        ("pause", "暂停"),
        ("resume", "继续"),
        ("toggle", "暂停 / 继续"),
        ("next", "下一首"),
        ("prev", "上一首"),
        ("stop", "停止播放"),
        ("status", "播放状态"),
        ("favorites", "本地收藏"),
        ("history", "最近播放"),
        ("doctor", "检查依赖和运行状态"),
    ]:
        command(name, help)
    command("seek", "相对跳转，单位秒").add_argument("seconds", type=float)
    command("volume", "设置音量 0–100").add_argument("value", type=int)
    command("mode", "播放模式").add_argument("value", choices=MODES)
    like = command("like", "收藏 / 取消收藏当前歌曲或指定 ID")
    like.add_argument("--id")
    queue = command("queue", "播放队列")
    queue_commands = queue.add_subparsers(dest="action", required=True)
    common(queue_commands.add_parser("list", help="显示队列"))
    add = common(queue_commands.add_parser("add", help="添加到队列"))
    song_arguments(add)
    add.add_argument("--next", action="store_true", help="插入为下一首")
    remove = common(queue_commands.add_parser("remove", help="移除第 N 首"))
    remove.add_argument("index", type=int)
    common(queue_commands.add_parser("clear", help="清空队列并停止播放"))
    playlist = command("playlist", "管理本地歌单")
    playlist_commands = playlist.add_subparsers(dest="action", required=True)
    common(playlist_commands.add_parser("list", help="列出歌单"))
    for action, help in [
        ("create", "新建歌单"),
        ("rename", "重命名收藏夹"),
        ("delete", "删除收藏夹及收录关系"),
        ("show", "显示歌曲"),
        ("add", "添加歌曲"),
        ("remove", "移除歌曲"),
        ("play", "播放歌单"),
    ]:
        sub = common(playlist_commands.add_parser(action, help=help))
        sub.add_argument("name")
        if action == "rename":
            sub.add_argument("new_name")
        if action in ("add", "remove"):
            sub.add_argument("--id", required=True)
    lyrics = command("lyrics", "查看或导出 LRC 歌词")
    lyrics.add_argument("--id")
    lyrics.add_argument("--output", type=Path)
    lyrics.add_argument("--force", action="store_true", help="覆盖已有歌词文件")
    lyrics.add_argument("--refresh", action="store_true", help="重新获取歌词并更新本地缓存")
    download = command("download", "下载音频，可选同时保存歌词")
    song_arguments(download)
    download.add_argument("--quality", choices=QUALITIES)
    download.add_argument("--output", type=Path, default=Path("downloads"), help="输出目录")
    download.add_argument("--force", action="store_true")
    download.add_argument("--lyrics", action="store_true", help="同时保存同名 LRC")
    config = command("config", "查看或设置配置")
    config.add_argument("key", nargs="?")
    config.add_argument("value", nargs="?")
    daemon = command("daemon", "后台服务管理")
    daemon.add_argument("action", choices=("start", "stop", "status"))
    return parser


def track_table(tracks, *, title=None, current=-1):
    table = Table(title=title, box=None, expand=True, padding=(0, 1), highlight=False)
    table.add_column("", width=2)
    table.add_column("#", justify="right", width=3, style="dim")
    table.add_column("歌曲", ratio=3, overflow="ellipsis")
    table.add_column("歌手", ratio=2, overflow="ellipsis")
    table.add_column("时长", width=5)
    table.add_column("ID", style="dim", overflow="ellipsis")
    for i, track in enumerate(tracks):
        table.add_row(
            "▶" if i == current else "",
            str(i + 1),
            track.title,
            track.artist,
            clock(track.duration),
            track.id,
        )
    console.print(table)


def report_status(state):
    if not state:
        console.print("后台服务未运行。运行 wocli 或 wocli play 开始听歌。")
        return
    labels = {
        "loading": "正在加载",
        "playing": "播放中",
        "paused": "已暂停",
        "stopped": "已停止",
        "error": "播放失败",
    }
    console.print(f"[cyan]{labels.get(state['phase'], state['phase'])}[/cyan]", end="  ")
    if track := state.get("track"):
        console.print(f"{track['artist']} · {track['title']}", markup=False)
        console.print(
            f"{clock(state['position'])} / {clock(state['duration'])}  "
            f"音质 {state['quality'] or '—'}  音量 {state['volume']}%  "
            f"队列 {state['index'] + 1}/{len(state['queue'])}"
        )
    else:
        console.print("暂无歌曲")
    if state.get("error"):
        errors.print(state["error"], markup=False, style="red")


async def run(args):
    paths = Paths.discover()
    settings = Settings(paths)
    store = Store(paths)
    provider = KuwoProvider(direct=args.direct or settings.values["direct"])
    client = Client(paths)

    async def by_id(identity):
        if identity.startswith("local:"):
            track = store.track(identity)
            if not track:
                raise MusicError("本地歌曲不在曲库中，请使用文件路径")
            return track
        identity = normalize_id(identity)
        track = store.track(identity) or await asyncio.to_thread(provider.details, identity)
        store.remember([track])
        return track

    async def choose():
        if args.id:
            return await by_id(args.id)
        keyword = " ".join(args.keyword)
        if not keyword:
            raise MusicError("请提供搜索关键词、--id 或本地音频路径")
        if await asyncio.to_thread(lambda: Path(keyword).expanduser().is_file()):
            track = Track.local(keyword)
            store.remember([track])
            return track
        _, tracks = await asyncio.to_thread(provider.search, keyword)
        if not tracks:
            raise MusicError("没有找到歌曲，请尝试其他歌名或歌手")
        store.remember(tracks)
        pick = args.pick
        if pick is None:
            if not sys.stdin.isatty() or args.json:
                raise MusicError("请用 --pick N 明确选择搜索结果，或用 --id 指定歌曲")
            track_table(tracks)
            pick = IntPrompt.ask(
                "播放第几首", choices=[str(i + 1) for i in range(len(tracks))], default=1
            )
        if not 1 <= pick <= len(tracks):
            raise MusicError(f"序号应在 1 到 {len(tracks)} 之间")
        return tracks[pick - 1]

    async def control(operation, **kwargs):
        return await client.request(operation, **kwargs)

    try:
        cmd = args.command
        if cmd == "search":
            if args.page < 1 or not 1 <= args.limit <= 100:
                raise MusicError("页码应大于 0，每页条数应为 1–100")
            total, tracks = await asyncio.to_thread(
                provider.search, " ".join(args.keyword), args.page - 1, args.limit
            )
            store.remember(tracks)
            data = {"total": total, "page": args.page, "tracks": [t.data() for t in tracks]}
            if not args.json:
                track_table(tracks, title=f"搜索结果 · 共 {total} 首 · 第 {args.page} 页")
                if not tracks:
                    console.print("没有找到歌曲，请尝试其他关键词。")
        elif cmd == "play":
            mpv_command()
            tracks = [await choose()] if args.id or args.keyword else []
            data = await control(
                "play",
                tracks=[t.data() for t in tracks],
                **({"index": 0} if tracks else {}),
                quality=args.quality,
            )
            if not args.no_wait:
                data = await client.wait_until_ready()
            if not args.json:
                report_status(data)
        elif cmd in ("pause", "resume", "toggle", "next", "prev", "stop", "seek", "volume", "mode"):
            kwargs = {}
            if cmd == "seek":
                kwargs["seconds"] = args.seconds
            if cmd in ("volume", "mode"):
                kwargs["value"] = args.value
            data = await control(cmd, **kwargs)
            if not args.json:
                report_status(data)
        elif cmd == "status":
            data = await client.request("status", autostart=False)
            if not args.json:
                report_status(data)
        elif cmd == "queue":
            if args.action == "add":
                track = await choose()
                data = await control("queue_add", tracks=[track.data()], next=args.next)
            elif args.action == "remove":
                data = await control("queue_remove", index=args.index - 1)
            elif args.action == "clear":
                data = await control("queue_clear")
            else:
                data = await client.request("status", autostart=False)
                if data is None:
                    data = store.load("queue", {"tracks": [], "index": -1})
                    data = {"queue": data["tracks"], "index": data["index"]}
            if not args.json:
                track_table(
                    [Track.from_data(t) for t in data["queue"]],
                    title="播放队列",
                    current=data["index"],
                )
        elif cmd == "like":
            track = await by_id(args.id) if args.id else None
            data = await control("like", track=track.data() if track else None)
            if not args.json:
                console.print("已更新本地收藏。")
        elif cmd in ("favorites", "history"):
            tracks = store.favorites() if cmd == "favorites" else store.history()
            data = [track.data() for track in tracks]
            if not args.json:
                track_table(tracks, title="本地收藏" if cmd == "favorites" else "最近播放")
        elif cmd == "playlist":
            if args.action == "list":
                data = store.playlists()
                if not args.json:
                    for item in data:
                        console.print(f"{item['name']}  ·  {item['count']} 首", markup=False)
                    if not data:
                        console.print('暂无歌单。使用 wocli playlist create "歌单名" 创建。')
            elif args.action == "create":
                store.create_playlist(args.name)
                data = {"name": args.name}
            elif args.action == "rename":
                store.rename_playlist(args.name, args.new_name)
                data = {"name": args.new_name, "previous": args.name}
            elif args.action == "delete":
                store.delete_playlist(args.name)
                data = {"deleted": args.name}
            elif args.action == "add":
                track = await by_id(args.id)
                store.add_to_playlist(args.name, track)
                data = {"name": args.name, "track": track.data()}
            elif args.action == "remove":
                identity = args.id if args.id.startswith("local:") else normalize_id(args.id)
                store.remove_from_playlist(args.name, identity)
                data = {"name": args.name, "removed": identity}
            else:
                tracks = store.playlist(args.name)
                if args.action == "play":
                    if not tracks:
                        raise MusicError("该歌单还没有歌曲")
                    mpv_command()
                    await control("play", tracks=[t.data() for t in tracks], index=0)
                    data = await client.wait_until_ready()
                    if not args.json:
                        report_status(data)
                else:
                    data = [track.data() for track in tracks]
                    if not args.json:
                        track_table(tracks, title=args.name)
            if not args.json and args.action in ("create", "add", "remove", "rename", "delete"):
                console.print("已更新本地歌单。")
        elif cmd == "lyrics":
            if args.id:
                track = await by_id(args.id)
            else:
                state = await client.request("status", autostart=False)
                if not state or not state["track"]:
                    raise MusicError("请先播放歌曲，或通过 --id 指定歌曲")
                track = Track.from_data(state["track"])
            text = "" if args.refresh else store.lyric(track.id)
            text = text or await asyncio.to_thread(provider.lyrics, track)
            if not text:
                raise MusicError("暂未找到匹配的歌词")
            store.lyric(track.id, text)
            if args.output:
                if args.output.exists() and not args.force:
                    raise MusicError("歌词文件已存在，使用 --force 覆盖")
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(text, encoding="utf8")
            data = {"id": track.id, "lyrics": text}
            if not args.json:
                console.print(str(args.output) if args.output else text, markup=False)
        elif cmd == "download":
            track = await choose()
            if track.path:
                raise MusicError("该歌曲已经是本地文件")
            stream = await asyncio.to_thread(
                provider.stream, track, args.quality or settings.values["quality"]
            )
            name = re.sub(r'[\\/:*?"<>|]', "_", f"{track.artist} - {track.title}").strip(". ")
            target = args.output.expanduser().resolve() / f"{name}{stream.extension}"
            with Progress(disable=args.json, console=errors) as progress:
                task = progress.add_task("下载音频", total=stream.size)
                cancelled = threading.Event()
                try:
                    await asyncio.to_thread(
                        provider.download,
                        stream,
                        target,
                        overwrite=args.force,
                        cancelled=cancelled,
                        progress=lambda done, total: progress.update(
                            task, total=total, completed=done
                        ),
                    )
                except asyncio.CancelledError:
                    cancelled.set()
                    raise
            if args.lyrics:
                text = await asyncio.to_thread(provider.lyrics, track)
                if text:
                    target.with_suffix(".lrc").write_text(text, encoding="utf8")
            data = {"path": str(target), "quality": stream.quality, "bytes": target.stat().st_size}
            if not args.json:
                console.print(f"已保存 {target}", markup=False)
        elif cmd == "config":
            if args.value is not None:
                value = args.value
                if args.key in ("volume", "lyric_offset"):
                    try:
                        value = int(value)
                    except ValueError as error:
                        raise MusicError("配置值应为整数") from error
                elif args.key == "direct":
                    if value.lower() not in ("true", "false"):
                        raise MusicError("direct 应为 true 或 false")
                    value = value.lower() == "true"
                state = await client.request("status", autostart=False)
                if state:
                    await control("config", key=args.key, value=value)
                else:
                    settings.set(args.key, value)
            data = Settings(paths).values
            if args.key:
                if args.key not in data:
                    raise MusicError(f"未知配置项：{args.key}")
                data = {args.key: data[args.key]}
            if not args.json:
                console.print_json(data=data)
        elif cmd == "daemon":
            if args.action == "start":
                data = await client.request("status")
            elif args.action == "stop":
                data = await client.request("shutdown", autostart=False)
            else:
                data = await client.request("status", autostart=False)
            if not args.json:
                console.print(
                    "后台服务已关闭。"
                    if args.action == "stop"
                    else ("后台服务运行中。" if data else "后台服务未运行。")
                )
        elif cmd == "doctor":
            try:
                backend = mpv_command()
            except MusicError:
                backend = None
            data = {
                "version": __version__,
                "python": sys.version.split()[0],
                "platform": sys.platform,
                "mpv": backend,
                "home": str(paths.home),
                "socket": str(paths.socket),
                "service_running": bool(await client.request("status", autostart=False)),
            }
            if not args.json:
                console.print_json(data=data)
                if not data["mpv"]:
                    console.print(
                        "安装播放器：brew install mpv（macOS） / sudo apt install mpv（Ubuntu）"
                    )
        else:
            raise MusicError("未知命令")
        if args.json:
            print(json.dumps(data, ensure_ascii=False))
    finally:
        store.close()


def main():
    args = make_parser().parse_args()
    try:
        if args.command in (None, "tui"):
            if args.json:
                raise MusicError("终端界面不支持 --json，请使用 search / status 等命令")
            if not sys.stdin.isatty() or not sys.stdout.isatty():
                raise MusicError(
                    "交互界面需要终端。脚本中请使用 wocli search / play / status --json"
                )
            from .tui import MusicApp

            MusicApp(direct=args.direct).run()
        else:
            asyncio.run(run(args))
    except (TimeoutError, MusicError, OSError) as error:
        if getattr(args, "json", False):
            print(
                json.dumps({"ok": False, "error": str(error)}, ensure_ascii=False), file=sys.stderr
            )
        else:
            errors.print(f"错误：{error}", style="red", markup=False)
        raise SystemExit(1) from None
    except KeyboardInterrupt:
        raise SystemExit(130) from None
