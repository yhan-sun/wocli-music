use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Search {
    /// Song title, artist, or both. Results contain IDs to use with play_music.
    pub keyword: String,
    #[serde(default = "one")]
    #[schemars(range(min = 1, max = 1000))]
    pub page: usize,
    #[serde(default = "twenty")]
    #[schemars(range(min = 1, max = 100))]
    pub limit: usize,
}
fn one() -> usize {
    1
}
fn twenty() -> usize {
    20
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Page {
    #[serde(default)]
    #[schemars(range(max = 100000))]
    pub offset: usize,
    #[serde(default = "twenty")]
    #[schemars(range(min = 1, max = 100))]
    pub limit: usize,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryKind {
    Favorites,
    History,
    Playlists,
    Playlist,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub kind: LibraryKind,
    /// Required when kind is playlist.
    pub name: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "twenty")]
    pub limit: usize,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
pub enum Quality {
    #[serde(rename = "best")]
    Best,
    #[serde(rename = "flac")]
    Flac,
    #[serde(rename = "320k")]
    High,
    #[serde(rename = "128k")]
    Standard,
}
impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::Flac => "flac",
            Self::High => "320k",
            Self::Standard => "128k",
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Play {
    /// Exact ID from search/library, e.g. kuwo:228908 or local:... .
    pub track_id: Option<String>,
    /// Local audio path. Cannot be combined with track_id.
    pub path: Option<String>,
    pub quality: Option<Quality>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Pause,
    Resume,
    Toggle,
    Stop,
    Next,
    Previous,
    Retry {
        quality: Option<Quality>,
    },
    /// Relative seek; negative seconds rewind.
    Seek {
        seconds: f64,
    },
    Volume {
        value: u8,
    },
    Mode {
        value: Mode,
    },
}
#[derive(Deserialize, JsonSchema, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    List,
    Repeat,
    Single,
    Shuffle,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Queue {
    Add {
        track_ids: Vec<String>,
        #[serde(default)]
        next: bool,
    },
    /// Queue positions are 1-based. track_id protects against a changed queue.
    Remove {
        index: usize,
        track_id: String,
    },
    Clear,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Playlist {
    Create {
        name: String,
    },
    Rename {
        name: String,
        new_name: String,
    },
    Delete {
        name: String,
    },
    Add {
        name: String,
        track_id: String,
    },
    Remove {
        name: String,
        track_id: String,
    },
    Play {
        name: String,
        quality: Option<Quality>,
    },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Favorite {
    /// Omit to use the current track.
    pub track_id: Option<String>,
    /// Explicit desired state; repeated calls do not toggle it.
    pub liked: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lyrics {
    /// Omit to use the current track.
    pub track_id: Option<String>,
    #[serde(default)]
    pub refresh: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Download {
    pub track_id: String,
    /// Absolute or ~-relative directory. Defaults to WOCLI_DOWNLOAD_DIR or ~/Music/wocli.
    pub directory: Option<String>,
    pub quality: Option<Quality>,
    #[serde(default)]
    pub lyrics: bool,
    #[serde(default)]
    pub overwrite: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum DownloadControl {
    Cancel { job_id: String },
    Retry { job_id: String },
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceControl {
    Enable { source_id: String },
    Disable { source_id: String },
    Prefer { source_id: String },
}
#[derive(Deserialize, JsonSchema)]
#[serde(
    tag = "key",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Config {
    Quality(Quality),
    Volume(u8),
    Mode(Mode),
    Direct(bool),
    LyricOffset(i64),
    LyricsBar(bool),
    Tray(bool),
}
