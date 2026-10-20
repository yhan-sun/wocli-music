import json
import sqlite3
import time

from .models import MusicError, Track


class Store:
    def __init__(self, paths):
        self.db = sqlite3.connect(paths.database, timeout=10)
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS tracks (id TEXT PRIMARY KEY, data TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS favorites (id TEXT PRIMARY KEY, added REAL NOT NULL);
            CREATE TABLE IF NOT EXISTS history (seq INTEGER PRIMARY KEY, id TEXT, played REAL);
            CREATE TABLE IF NOT EXISTS playlists (name TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS members (
                name TEXT, id TEXT, position INTEGER, PRIMARY KEY(name, id));
            CREATE TABLE IF NOT EXISTS state (key TEXT PRIMARY KEY, data TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS lyrics (id TEXT PRIMARY KEY, text TEXT NOT NULL);
        """)

    def close(self):
        self.db.close()

    def remember(self, tracks: list[Track]):
        with self.db:
            self.db.executemany(
                "INSERT OR REPLACE INTO tracks VALUES (?,?)",
                [(t.id, json.dumps(t.data(), ensure_ascii=False)) for t in tracks],
            )

    def track(self, identity: str) -> Track | None:
        row = self.db.execute("SELECT data FROM tracks WHERE id=?", (identity,)).fetchone()
        return Track.from_data(json.loads(row[0])) if row else None

    def favorite(self, track: Track) -> bool:
        self.remember([track])
        exists = self.db.execute("SELECT 1 FROM favorites WHERE id=?", (track.id,)).fetchone()
        with self.db:
            if exists:
                self.db.execute("DELETE FROM favorites WHERE id=?", (track.id,))
            else:
                self.db.execute("INSERT INTO favorites VALUES (?,?)", (track.id, time.time()))
        return not exists

    def is_favorite(self, identity: str) -> bool:
        return bool(self.db.execute("SELECT 1 FROM favorites WHERE id=?", (identity,)).fetchone())

    def favorites(self) -> list[Track]:
        rows = self.db.execute("""SELECT t.data FROM favorites f JOIN tracks t ON t.id=f.id
                                  ORDER BY f.added DESC""")
        return [Track.from_data(json.loads(row[0])) for row in rows]

    def played(self, track: Track):
        self.remember([track])
        with self.db:
            self.db.execute("INSERT INTO history(id,played) VALUES (?,?)", (track.id, time.time()))
            self.db.execute(
                "DELETE FROM history WHERE seq NOT IN "
                "(SELECT seq FROM history ORDER BY seq DESC LIMIT 1000)"
            )

    def history(self) -> list[Track]:
        rows = self.db.execute("""SELECT t.data FROM history h JOIN tracks t ON t.id=h.id
                                  GROUP BY h.id ORDER BY MAX(h.played) DESC LIMIT 100""")
        return [Track.from_data(json.loads(row[0])) for row in rows]

    def save(self, key: str, value):
        with self.db:
            self.db.execute("INSERT OR REPLACE INTO state VALUES (?,?)", (key, json.dumps(value)))

    def load(self, key: str, default=None):
        row = self.db.execute("SELECT data FROM state WHERE key=?", (key,)).fetchone()
        return json.loads(row[0]) if row else default

    def lyric(self, identity: str, text: str | None = None) -> str:
        if text is not None:
            with self.db:
                self.db.execute("INSERT OR REPLACE INTO lyrics VALUES (?,?)", (identity, text))
            return text
        row = self.db.execute("SELECT text FROM lyrics WHERE id=?", (identity,)).fetchone()
        return row[0] if row else ""

    def playlists(self) -> list[dict]:
        return [
            {"name": name, "count": count}
            for name, count in self.db.execute(
                "SELECT p.name,COUNT(m.id) FROM playlists p LEFT JOIN members m ON p.name=m.name "
                "GROUP BY p.name ORDER BY p.name"
            )
        ]

    def create_playlist(self, name: str):
        if not name.strip() or len(name) > 100:
            raise MusicError("歌单名称应为 1 到 100 个字符")
        with self.db:
            self.db.execute("INSERT OR IGNORE INTO playlists VALUES (?)", (name,))

    def playlist(self, name: str) -> list[Track]:
        if not self.db.execute("SELECT 1 FROM playlists WHERE name=?", (name,)).fetchone():
            raise MusicError(f"找不到本地歌单：{name}")
        rows = self.db.execute(
            "SELECT t.data FROM members m JOIN tracks t ON m.id=t.id "
            "WHERE m.name=? ORDER BY m.position",
            (name,),
        )
        return [Track.from_data(json.loads(row[0])) for row in rows]

    def add_to_playlist(self, name: str, track: Track):
        self.create_playlist(name)
        self.remember([track])
        with self.db:
            self.db.execute(
                "INSERT OR IGNORE INTO members VALUES (?,?,"
                "(SELECT COALESCE(MAX(position),0)+1 FROM members WHERE name=?))",
                (name, track.id, name),
            )

    def remove_from_playlist(self, name: str, identity: str):
        with self.db:
            self.db.execute("DELETE FROM members WHERE name=? AND id=?", (name, identity))
