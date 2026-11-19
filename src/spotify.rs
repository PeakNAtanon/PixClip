use crate::{media, process::Context, tools};
use anyhow::{Context as _, Result, ensure};
use serde_json::Value;
use std::path::Path;

pub struct Link {
    kind: Kind,
    id: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Track,
    Playlist,
    Album,
}

impl Link {
    pub fn parse(input: &str) -> Result<Option<Self>> {
        let Ok(url) = url::Url::parse(input) else {
            return Ok(None);
        };
        if url.host_str() != Some("open.spotify.com") {
            return Ok(None);
        }
        ensure!(
            matches!(url.scheme(), "https" | "http"),
            "Use an https Spotify link"
        );
        let mut parts = url.path_segments().into_iter().flatten();
        let first = parts.next().unwrap_or_default();
        let kind = if first.starts_with("intl-") {
            parts.next().unwrap_or_default()
        } else {
            first
        };
        let kind = match kind {
            "track" => Kind::Track,
            "playlist" => Kind::Playlist,
            "album" => Kind::Album,
            _ => anyhow::bail!("Use a Spotify song, playlist or album link"),
        };
        let id = parts
            .next()
            .filter(|id| valid_id(id, 22))
            .context("Invalid Spotify song, playlist or album link")?;
        Ok(Some(Self {
            kind,
            id: id.into(),
        }))
    }

    pub fn is_collection(&self) -> bool {
        self.kind != Kind::Track
    }

    pub fn fetch(&self, ctx: &Context) -> Result<(String, Vec<Track>)> {
        let kind = match self.kind {
            Kind::Track => "track",
            Kind::Playlist => "playlist",
            Kind::Album => "album",
        };
        let url = format!("https://open.spotify.com/embed/{kind}/{}", self.id);
        let html = tools::request_bytes(&url, None, ctx)
            .context("Could not read the public Spotify page")?;
        let html = std::str::from_utf8(&html)?;
        parse_embed(html, self.is_collection())
    }
}

fn valid_id(id: &str, length: usize) -> bool {
    id.len() == length
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || (length == 11 && b"_-".contains(&b)))
}

#[derive(Clone)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub duration: Option<f64>,
}

fn parse_embed(html: &str, collection: bool) -> Result<(String, Vec<Track>)> {
    let json = html
        .split_once("id=\"__NEXT_DATA__\"")
        .and_then(|(_, rest)| rest.split_once('>'))
        .and_then(|(_, rest)| rest.split_once("</script>"))
        .map(|(json, _)| json)
        .context("Spotify did not return track information. Check that the link is public.")?;
    let value: Value = serde_json::from_str(json)?;
    let entry = &value["props"]["pageProps"]["state"]["data"]["entity"];
    let title = entry["title"]
        .as_str()
        .or_else(|| entry["name"].as_str())
        .context("Spotify did not return a title for this link")?;
    let entries = if collection {
        entry["trackList"]
            .as_array()
            .context("Spotify did not return the public track list")?
            .as_slice()
    } else {
        std::slice::from_ref(entry)
    };
    let tracks: Vec<_> = entries
        .iter()
        .filter_map(|song| {
            let uri = song["uri"].as_str()?;
            if !uri.starts_with("spotify:track:") {
                return None;
            }
            let title = song["title"].as_str().or_else(|| song["name"].as_str())?;
            let artist = song["subtitle"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    song["artists"].as_array().map(|artists| {
                        artists
                            .iter()
                            .filter_map(|artist| artist["name"].as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                })
                .unwrap_or_default();
            Some(Track {
                title: title.into(),
                artist,
                duration: song["duration"].as_f64().map(|ms| ms / 1000.0),
            })
        })
        .collect();
    ensure!(
        !tracks.is_empty(),
        "No songs were visible in the public Spotify page"
    );
    Ok((title.into(), tracks))
}

pub struct Candidate {
    pub url: String,
    pub label: String,
}

pub fn search(track: &Track, yt_dlp: &Path, ctx: &Context) -> Result<Vec<Candidate>> {
    let keywords: String = format!("{} {} official audio", track.artist, track.title)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect();
    let query = format!("ytsearch3:{keywords}");
    let result = ctx.run(
        yt_dlp,
        &media::args(&[
            "--ignore-config",
            "--no-warnings",
            "--skip-download",
            "--flat-playlist",
            "--dump-single-json",
            "--socket-timeout",
            "10",
            "--retries",
            "1",
            "--extractor-retries",
            "1",
            "--",
            &query,
        ]),
        true,
        false,
    )?;
    ensure!(result.code == 0, "Could not search YouTube for this track");
    let value: Value = result
        .text
        .lines()
        .find_map(|line| serde_json::from_str(line).ok())
        .context("YouTube search did not return a result list")?;
    Ok(value["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let id = entry["id"].as_str().filter(|id| valid_id(id, 11))?;
            let title = entry["title"].as_str()?;
            let channel = entry["channel"]
                .as_str()
                .or_else(|| entry["uploader"].as_str())
                .unwrap_or("YouTube");
            let length = duration_label(entry["duration"].as_f64());
            Some(Candidate {
                url: format!("https://www.youtube.com/watch?v={id}"),
                label: format!("{title} • {channel}{length}"),
            })
        })
        .collect())
}

pub fn duration_label(seconds: Option<f64>) -> String {
    seconds
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|seconds| {
            let seconds = seconds.round() as u64;
            format!(" • {}:{:02}", seconds / 60, seconds % 60)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_album_link_loads_ordered_tracks() {
        let link =
            Link::parse("https://open.spotify.com/intl-th/album/6lvlRdaGuZsY6FJ3O4wAtd?si=test")
                .unwrap()
                .unwrap();
        let entity = serde_json::json!({
            "type": "album",
            "title": "Example album",
            "trackList": [
                {"uri": "spotify:track:first", "title": "เพลงแรก", "subtitle": "Artist", "duration": 247786},
                {"uri": "spotify:track:second", "title": "เพลงที่สอง", "subtitle": "Artist", "duration": 180000}
            ]
        });
        let html = format!(
            "<script id=\"__NEXT_DATA__\" type=\"application/json\">{}</script>",
            serde_json::json!({"props":{"pageProps":{"state":{"data":{"entity":entity}}}}})
        );
        assert!(link.kind == Kind::Album);
        let (title, tracks) = parse_embed(&html, link.is_collection()).unwrap();
        assert_eq!(title, "Example album");
        assert_eq!(tracks[0].title, "เพลงแรก");
        assert_eq!(tracks[0].artist, "Artist");
        assert_eq!(tracks[0].duration, Some(247.786));
        assert_eq!(tracks[1].title, "เพลงที่สอง");
    }

    #[test]
    fn public_spotify_metadata_preserves_track_and_playlist_names() {
        let track = serde_json::json!({
            "title": "Example song", "artists": [{"name": "Example artist"}],
            "duration": 180500, "uri": "spotify:track:example"
        });
        let playlist = serde_json::json!({"title":"Example playlist", "trackList": [
            {"title":"First", "subtitle":"A", "uri":"spotify:track:first"},
            {"title":"Episode", "uri":"spotify:episode:show"},
            {"title":"Second", "subtitle":"B", "uri":"spotify:track:second"}
        ]});
        let html = |entity| {
            format!(
                "<script id=\"__NEXT_DATA__\" type=\"application/json\">{}</script>",
                serde_json::json!({"props":{"pageProps":{"state":{"data":{"entity":entity}}}}})
            )
        };
        let (_, tracks) = parse_embed(&html(track), false).unwrap();
        assert_eq!(tracks[0].artist, "Example artist");
        assert_eq!(tracks[0].duration, Some(180.5));
        let (title, tracks) = parse_embed(&html(playlist), true).unwrap();
        assert_eq!(title, "Example playlist");
        assert_eq!(
            tracks
                .iter()
                .map(|song| song.title.as_str())
                .collect::<Vec<_>>(),
            ["First", "Second"]
        );
        let link =
            Link::parse("https://open.spotify.com/intl-th/track/4uLU6hMCjMI75M1A2tKUQC?si=123")
                .unwrap()
                .unwrap();
        assert!(!link.is_collection());
        assert!(
            Link::parse("https://open.spotify.com.example.com/track/notSpotify")
                .unwrap()
                .is_none()
        );
    }
}
