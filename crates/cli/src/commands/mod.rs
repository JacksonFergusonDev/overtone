pub mod roots;
pub mod tracks;
pub mod vibes;

use overtone_core::CoreError;
use overtone_core::library::{Library, LibraryError};
use overtone_core::model::{Track, TrackId, VibePath};

/// Resolves a track given as a numeric id or an exact file path.
pub fn resolve_track(library: &Library, selector: &str) -> Result<Track, CoreError> {
    if let Ok(id) = selector.parse::<i64>() {
        return Ok(library.track(TrackId(id))?);
    }
    let absolute = std::path::absolute(selector)
        .map_or_else(|_| selector.to_string(), |p| p.display().to_string());
    library.track_by_path(&absolute)?.ok_or_else(|| {
        CoreError::Library(LibraryError::NotFoundByName {
            kind: "track at path",
            name: absolute.clone(),
        })
    })
}

pub fn parse_vibe(input: &str) -> Result<VibePath, CoreError> {
    VibePath::parse(input).map_err(|err| CoreError::Library(LibraryError::InvalidVibe(err)))
}

pub fn find_vibe(
    library: &Library,
    path: &VibePath,
) -> Result<overtone_core::model::Vibe, CoreError> {
    library.vibe_by_path(path)?.ok_or_else(|| {
        CoreError::Library(LibraryError::NotFoundByName {
            kind: "vibe",
            name: path.to_string(),
        })
    })
}
