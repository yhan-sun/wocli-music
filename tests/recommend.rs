use wocli_music::{
    model::Track,
    provider::Provider,
    recommend,
    store::{Paths, Store},
};

fn track(id: usize, title: &str, artist: &str) -> Track {
    Track {
        id: format!("kuwo:{id}"),
        title: title.into(),
        artist: artist.into(),
        ..Track::default()
    }
}

#[test]
fn offline_daily_snapshot_and_feedback() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        home: dir.path().into(),
        runtime: dir.path().join("run"),
    };
    let store = Store::open(&paths).unwrap();
    let provider = Provider::new(false);
    let first = track(1, "第一首", "甲");
    let second = track(2, "第二首", "乙");
    store.set_favorite(&first, true).unwrap();
    store.create_folder("旧歌单").unwrap();
    store.collect("旧歌单", &second).unwrap();
    let mix = recommend::daily(&store, &provider, true, false).unwrap();
    assert_eq!(mix.items.len(), 2);
    assert!(mix.items.iter().any(|i| i.reason == "来自我喜欢"));
    let later = track(3, "第三首", "丙");
    store.set_favorite(&later, true).unwrap();
    let cached = recommend::daily(&store, &provider, true, false).unwrap();
    assert_eq!(
        serde_json::to_value(&mix).unwrap(),
        serde_json::to_value(&cached).unwrap()
    );
    recommend::set_blocked(&store, &first.id, true).unwrap();
    assert!(!recommend::daily(&store, &provider, true, false)
        .unwrap()
        .items
        .iter()
        .any(|i| i.track.id == first.id));
    recommend::set_blocked(&store, &first.id, false).unwrap();
    assert!(recommend::daily(&store, &provider, true, false)
        .unwrap()
        .items
        .iter()
        .any(|i| i.track.id == first.id));
    let refreshed = recommend::daily(&store, &provider, true, true).unwrap();
    assert!(refreshed.items.iter().any(|i| i.track.id == later.id));
}

#[test]
fn deduplicates_and_limits_artists() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&Paths {
        home: dir.path().into(),
        runtime: dir.path().join("run"),
    })
    .unwrap();
    for i in 1..=6 {
        store
            .set_favorite(&track(i, &format!("歌{i}"), "甲"), true)
            .unwrap();
    }
    store.set_favorite(&track(10, "歌1", "甲"), true).unwrap();
    let mix = recommend::daily(&store, &Provider::new(false), true, false).unwrap();
    assert_eq!(mix.items.len(), 3);
    let titles: std::collections::HashSet<_> = mix.items.iter().map(|i| &i.track.title).collect();
    assert_eq!(titles.len(), mix.items.len());
}
