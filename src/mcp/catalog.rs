use super::params::*;
use rmcp::model::{Tool, ToolAnnotations};
use schemars::JsonSchema;
use std::sync::{Arc, OnceLock};

fn tool<T: JsonSchema>(
    name: &'static str,
    description: &'static str,
    read: bool,
    destructive: bool,
    idempotent: bool,
    network: bool,
) -> Tool {
    let mut schema = schemars::schema_for!(T).to_value();
    schema["type"] = serde_json::json!("object");
    Tool::new(
        name,
        description,
        Arc::new(schema.as_object().unwrap().clone()),
    )
    .with_annotations(
        ToolAnnotations::new()
            .read_only(read)
            .destructive(destructive)
            .idempotent(idempotent)
            .open_world(network),
    )
}
pub fn tools(read_only: bool) -> Vec<Tool> {
    static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
    TOOLS.get_or_init(||vec![
        tool::<Search>("search_music","Search the Kuwo music catalog. Return candidates with exact track IDs; do not guess which song the user meant when results are ambiguous.",true,false,true,true),
        tool::<Empty>("get_playback_status","Read playback state, current track, position, source, volume and queue length. Does not start the audio daemon.",true,false,true,false),
        tool::<Page>("get_queue","Read a page of the queue and the current 1-based position, including while the audio daemon is stopped.",true,false,true,false),
        tool::<Library>("get_library","Read favorites, listening history, playlist names, or tracks in a named playlist. Results are paginated.",true,false,true,false),
        tool::<Lyrics>("get_lyrics","Read cached lyrics for a track or the current song; fetch from the music provider if missing or refresh is true.",true,false,true,true),
        tool::<Page>("get_downloads","Read background download jobs and progress without waiting for completion or starting the daemon.",true,false,true,false),
        tool::<Empty>("list_music_sources","List music source IDs, order and enabled state. Script bodies and import URLs are omitted. Import new sources with the CLI.",true,false,true,false),
        tool::<Empty>("get_player_config","Read quality, volume, playback mode and other saved preferences.",true,false,true,false),
        tool::<Play>("play_music","Play an exact track ID or a local audio path. With no ID/path, restart the current queue item. Returns loading status immediately; poll get_playback_status for failures. Starts the existing daemon if necessary.",false,false,false,true),
        tool::<Control>("control_playback","Pause, resume, stop, change tracks, retry sources, seek relative seconds, set volume 0-100 or playback mode. Loading/cutting tracks return immediately. volume/mode persist.",false,false,false,true),
        tool::<Queue>("manage_queue","Add up to 100 exact IDs, remove a 1-based position with a matching track_id, or clear the queue. Removing the playing item changes playback; clear stops playback.",false,true,false,true),
        tool::<Playlist>("manage_playlist","Create, rename, delete or edit a playlist, or play its tracks. Delete removes the playlist, not audio files. Play replaces the current queue.",false,true,false,true),
        tool::<Favorite>("set_favorite","Set whether an exact track (or the current song) is liked. This operation sets the requested state and does not toggle on retries.",false,true,true,true),
        tool::<Download>("download_music","Enqueue a song download with optional lyrics. Returns job_id immediately; poll get_downloads. Overwrite defaults to false. Uses the same source fallback as the player.",false,true,false,true),
        tool::<DownloadControl>("manage_download","Cancel or retry a download job by ID. A retry runs in the background; query get_downloads for the result.",false,true,false,true),
        tool::<Config>("configure_player","Persist a typed player setting. Volume is 0-100; lyric_offset is milliseconds within -60000..60000. Does not start a stopped daemon.",false,true,true,false),
        tool::<SourceControl>("manage_music_sources","Enable/disable an existing music source or prefer it (enable and move to first place). Applies to subsequent playback/download attempts. Does not import or execute new scripts.",false,true,true,false),
    ]).iter().filter(|t|!read_only||t.annotations.as_ref().is_some_and(|a|a.read_only_hint==Some(true))).cloned().collect()
}
