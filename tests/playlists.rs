use flate2::{write::GzEncoder, Compression};
use std::{
    fs,
    io::Write,
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};
use tempfile::TempDir;
use wocli_music::{
    model::Track,
    playlists::{self, Conflict, Entry, MatchState, Playlist, ResolveOptions},
    store::{Paths, Store},
};

fn setup() -> (TempDir, Paths, Store) {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths {
        home: root.path().into(),
        runtime: root.path().join("run"),
    };
    let store = Store::open(&paths).unwrap();
    (root, paths, store)
}
fn file(root: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = root.join(name);
    fs::write(&path, content).unwrap();
    path
}
fn entry(title: &str) -> Entry {
    Entry {
        title: title.into(),
        artist: "歌手".into(),
        ..Entry::default()
    }
}
fn pending(name: &str, title: &str) -> Playlist {
    Playlist {
        name: name.into(),
        format: "test".into(),
        entries: vec![entry(title)],
    }
}
fn track(id: &str) -> Track {
    Track {
        id: format!("kuwo:{id}"),
        title: "歌曲".into(),
        artist: "歌手".into(),
        duration: 180.0,
        ..Track::default()
    }
}
fn cli(paths: &Paths, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_wocli"))
        .arg("--home")
        .arg(&paths.home)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn csv_handles_chinese_bom_quoted_header_newlines_and_units() {
    let (root, _, _) = setup();
    let path=file(root.path(),"曲目.csv","\u{feff}\"歌曲名\",歌手,专辑,时长毫秒,平台,歌曲ID\r\n\"歌,曲\n第二行\",\"歌手\",专辑,183000,网易云,123\r\n");
    let lists = playlists::read(&path, "artist-title").unwrap();
    let e = &lists[0].entries[0];
    assert_eq!(e.title, "歌,曲\n第二行");
    assert_eq!(e.duration, 183.0);
    assert_eq!(e.source, "wy");
    assert_eq!(e.id, "123");
    let bad = file(root.path(), "bad.csv", "title,artist\n\"a,b\",c,d");
    assert!(playlists::read(&bad, "artist-title").is_err());
}
#[test]
fn domestic_backup_schemas_gzip_and_multiple_sheet_selection() {
    // Synthetic fixtures follow the official fixed-commit TypeScript schemas in docs/playlist-import-design.md.
    let (root, _, _) = setup();
    let lx = r#"{"type":"playListPart_v2","data":{"id":"list_1","name":"LX 收藏","list":[{"id":"wy_123","name":"歌曲","singer":"歌手","source":"wy","interval":"03:01","meta":{"songId":123,"albumName":"专辑"}}]}}"#;
    let path = root.path().join("收藏.lxmc");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(lx.as_bytes()).unwrap();
    fs::write(&path, encoder.finish().unwrap()).unwrap();
    let list =
        playlists::select(playlists::read(&path, "artist-title").unwrap(), None, None).unwrap();
    assert_eq!(list.name, "LX 收藏");
    assert_eq!(list.entries[0].id, "123");
    assert_eq!(list.entries[0].source, "wy");
    assert_eq!(list.entries[0].duration, 181.0);
    let mf = file(
        root.path(),
        "mf.json",
        r#"{"musicSheets":[{"title":"收藏","musicList":[{"id":"123","platform":"QQ音乐","title":"歌曲","artist":"歌手","album":"专辑","duration":180}]},{"title":"其他","musicList":[]}],"plugins":[{"srcUrl":"https://invalid.example/never-execute.js","version":"1"}]}"#,
    );
    let lists = playlists::read(&mf, "artist-title").unwrap();
    assert_eq!(lists.len(), 2);
    assert!(playlists::select(lists.clone(), None, None).is_err());
    let list = playlists::select(lists, Some("1"), Some("新的名字")).unwrap();
    assert_eq!(list.name, "新的名字");
    assert_eq!(list.entries[0].source, "tx");
    let unsupported = file(
        root.path(),
        "legacy.json",
        r#"{"type":"playList","data":[]}"#,
    );
    assert!(playlists::read(&unsupported, "artist-title").is_err());
}
#[test]
fn local_playlists_resolve_relative_unicode_paths_and_reject_hls() {
    let (root, paths, _) = setup();
    file(root.path(), "歌 曲.mp3", "dummy");
    let path = file(
        root.path(),
        "local.m3u8",
        "#EXTM3U\n#EXTINF:180,歌手 - 歌曲\n歌 曲.mp3\n#EXTINF:180,歌手 - 不存在\nmissing.mp3\n",
    );
    let mut list = playlists::read(&path, "artist-title").unwrap().remove(0);
    playlists::resolve(
        &mut list,
        &[],
        &paths,
        ResolveOptions {
            offline: true,
            ..ResolveOptions::default()
        },
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(list.summary().matched, 1);
    assert_eq!(list.summary().pending, 1);
    assert_eq!(list.entries[0].resolved.as_ref().unwrap().artist, "歌手");
    assert!(Path::new(&list.entries[0].path).is_absolute());
    let hls = file(
        root.path(),
        "hls.m3u8",
        "#EXTM3U\n#EXT-X-TARGETDURATION:6\nchunk.ts",
    );
    assert!(playlists::read(&hls, "artist-title").is_err());
    let pls=file(root.path(),"local.pls","[playlist]\nFile2=missing.mp3\nTitle2=歌手 - 第二首\nFile1=歌 曲.mp3\nTitle1=歌手 - 第一首\nLength1=180\n");
    let list = playlists::read(&pls, "artist-title").unwrap().remove(0);
    assert_eq!(list.entries[0].title, "第一首");
    let uri=file(root.path(),"uri.jspf",&serde_json::json!({"playlist":{"title":"URI","track":[{"title":"歌曲","duration":180000,"location":[format!("file://{}/%E6%AD%8C%20%E6%9B%B2.mp3",root.path().canonicalize().unwrap().display())]},{"title":"bad percent","location":["file:///x%歌"]}]}}).to_string());
    let list = playlists::read(&uri, "artist-title").unwrap().remove(0);
    assert!(list.entries[0].path.ends_with("歌 曲.mp3"));
    assert_eq!(list.entries[0].duration, 180.0);
}
#[test]
fn text_order_is_explicit_and_preserves_hyphenated_titles() {
    let (root, _, _) = setup();
    let path = file(root.path(), "歌曲.txt", "歌手 - 歌名 - Part II\n单独歌名");
    let a = playlists::read(&path, "artist-title").unwrap();
    assert_eq!(a[0].entries[0].title, "歌名 - Part II");
    let b = playlists::read(&path, "title-artist").unwrap();
    assert_eq!(b[0].entries[0].title, "歌手");
    assert!(playlists::read(&path, "guess").is_err());
}
#[test]
fn native_roundtrip_preserves_provenance_pending_candidates_and_duplicates() {
    let (root, paths, store) = setup();
    let mut a = Entry::from_track(track("100"));
    a.source = "wy".into();
    a.id = "foreign-100".into();
    let mut b = a.clone();
    b.source = "tx".into();
    b.id = "qq-100".into();
    let mut c = entry("待确认");
    c.status = MatchState::Ambiguous;
    c.reason = "多个候选".into();
    c.candidates = vec![track("200")];
    let mut list = Playlist {
        name: "往返".into(),
        format: "lx-v2".into(),
        entries: vec![a, b, c, entry("未匹配")],
    };
    list.deduplicate();
    playlists::save(&store, list.clone(), Conflict::Error).unwrap();
    assert_eq!(store.folder("往返").unwrap().len(), 1);
    assert_eq!(playlists::stored(&store, "往返").unwrap(), list);
    let path = root.path().join("backup.json");
    playlists::write(&list, "json", &path, false).unwrap();
    let mut reread = playlists::read(&path, "artist-title").unwrap().remove(0);
    assert_eq!(reread, list);
    playlists::resolve(
        &mut reread,
        &[],
        &paths,
        ResolveOptions {
            offline: true,
            ..ResolveOptions::default()
        },
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(reread.entries[0].source, "wy");
    assert_eq!(reread.entries[1].source, "tx");
    assert_eq!(reread.entries[1].id, "qq-100");
    assert!(playlists::write(&list, "json", &path, false).is_err());
    assert_eq!(playlists::read(&path, "artist-title").unwrap()[0], list);
    let csv = root.path().join("backup.csv");
    playlists::write(&list, "csv", &csv, false).unwrap();
    let rows = playlists::read(&csv, "artist-title").unwrap();
    assert_eq!(rows[0].entries[0].id, "foreign-100");
    assert!(playlists::write(&list, "m3u8", &root.path().join("bad.m3u8"), false).is_err());
}
#[test]
fn conflicts_snapshots_and_ordinary_edits_remain_consistent() {
    let (_, _, store) = setup();
    playlists::save(&store, pending("收藏", "旧歌曲"), Conflict::Error).unwrap();
    assert!(playlists::save(&store, pending("收藏", "不应写入"), Conflict::Error).is_err());
    assert_eq!(
        playlists::stored(&store, "收藏").unwrap().entries[0].title,
        "旧歌曲"
    );
    let renamed = playlists::save(&store, pending("收藏", "另存"), Conflict::Rename).unwrap();
    assert_eq!(renamed.name, "收藏 (导入 2)");
    let merged = playlists::save(&store, pending("收藏", "新歌曲"), Conflict::Merge).unwrap();
    assert!(merged.backup.is_some());
    assert_eq!(merged.summary.total, 2);
    let replaced = playlists::save(&store, pending("收藏", "替换歌曲"), Conflict::Replace).unwrap();
    assert_eq!(replaced.summary.total, 1);
    playlists::restore(&store, "收藏", replaced.backup.unwrap()).unwrap();
    assert_eq!(playlists::stored(&store, "收藏").unwrap().entries.len(), 2);
    store.collect("收藏", &track("100")).unwrap();
    assert_eq!(playlists::stored(&store, "收藏").unwrap().entries.len(), 3);
    store.remove("收藏", "kuwo:100").unwrap();
    assert_eq!(playlists::stored(&store, "收藏").unwrap().entries.len(), 2);
    store.rename("收藏", "改名").unwrap();
    assert_eq!(playlists::stored(&store, "改名").unwrap().name, "改名");
    assert!(!playlists::backups(&store, "改名").unwrap().is_empty());
    store.delete("改名").unwrap();
    store.create_folder("改名").unwrap();
    assert!(playlists::stored(&store, "改名")
        .unwrap()
        .entries
        .is_empty());
}
#[test]
fn cancelled_resolution_does_not_mutate_plan_or_store() {
    let (_, paths, store) = setup();
    let mut list = pending("取消", "歌曲");
    let original = list.clone();
    let cancel = AtomicBool::new(true);
    let err = playlists::resolve(
        &mut list,
        &[],
        &paths,
        ResolveOptions {
            offline: true,
            ..ResolveOptions::default()
        },
        &cancel,
        |_, _| {},
    )
    .unwrap_err();
    assert_eq!(err.code, "cancelled");
    assert_eq!(list, original);
    assert!(store.folders().unwrap().is_empty());
    cancel.store(false, Ordering::Relaxed);
    playlists::resolve(
        &mut list,
        &[track("1")],
        &paths,
        ResolveOptions {
            offline: true,
            ..ResolveOptions::default()
        },
        &cancel,
        |_, _| {
            cancel.store(true, Ordering::Relaxed);
        },
    )
    .unwrap_err();
    assert_eq!(list, original);
}
#[test]
fn bounds_versions_and_invalid_metadata_fail_without_replacing_a_playlist() {
    let (root, _, store) = setup();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(&vec![b' '; playlists::MAX_BYTES + 1])
        .unwrap();
    let path = root.path().join("large.lxmc");
    fs::write(&path, encoder.finish().unwrap()).unwrap();
    assert!(playlists::read(&path, "artist-title")
        .unwrap_err()
        .message
        .contains("16 MiB"));
    let future = file(
        root.path(),
        "future.json",
        r#"{"format":"wocli-playlist","version":99,"playlists":[]}"#,
    );
    assert!(playlists::read(&future, "artist-title").is_err());
    playlists::save(&store, pending("已有", "原曲"), Conflict::Error).unwrap();
    let mut bad = pending("已有", "坏数据");
    bad.entries[0].duration = f64::NAN;
    assert!(playlists::save(&store, bad, Conflict::Replace).is_err());
    assert_eq!(
        playlists::stored(&store, "已有").unwrap().entries[0].title,
        "原曲"
    );
    let mut big = pending("已有", "超过限制");
    big.entries = vec![entry("歌"); playlists::MAX_ENTRIES + 1];
    assert!(playlists::save(&store, big, Conflict::Replace).is_err());
}
#[test]
fn cli_dry_run_import_rename_export_reimport_and_manual_repair() {
    let (root, paths, store) = setup();
    store.remember(&[track("77")]).unwrap();
    let path = file(
        root.path(),
        "foreign.csv",
        "title,artist,platform,trackId\n歌曲,歌手,wy,77\n缺失,歌手,tx,77\n",
    );
    let input = path.to_str().unwrap();
    let preview = cli(
        &paths,
        &[
            "playlist",
            "import",
            input,
            "--name",
            "通勤",
            "--offline",
            "--dry-run",
            "--json",
        ],
    );
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(store.folders().unwrap().is_empty());
    let p: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(p["summary"]["matched"], 1);
    assert_eq!(p["summary"]["pending"], 1);
    let out = cli(
        &paths,
        &[
            "playlist",
            "import",
            input,
            "--name",
            "通勤",
            "--offline",
            "--json",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(store.folder("通勤").unwrap().len(), 1);
    assert!(cli(
        &paths,
        &["playlist", "match", "通勤", "--entry", "2", "--id", "77", "--json"]
    )
    .status
    .success());
    assert_eq!(
        playlists::stored(&store, "通勤")
            .unwrap()
            .summary()
            .duplicates,
        1
    );
    let backup = root.path().join("通勤.json");
    assert!(cli(
        &paths,
        &[
            "playlist",
            "export",
            "通勤",
            "--output",
            backup.to_str().unwrap(),
            "--json"
        ]
    )
    .status
    .success());
    assert!(cli(
        &paths,
        &[
            "playlist",
            "import",
            backup.to_str().unwrap(),
            "--offline",
            "--json"
        ]
    )
    .status
    .success());
    assert!(store.folder("通勤 (导入 2)").is_ok());
    assert!(cli(
        &paths,
        &["playlist", "drop", "通勤", "--entry", "2", "--json"]
    )
    .status
    .success());
    assert_eq!(playlists::stored(&store, "通勤").unwrap().entries.len(), 1);
}
