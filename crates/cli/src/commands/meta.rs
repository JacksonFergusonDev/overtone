use clap::Subcommand;
use overtone_core::CoreError;
use overtone_core::library::{Library, LibraryError};
use overtone_core::model::{
    FieldSource, FieldValue, MetadataField, MetadataSuggestion, SuggestionId, SuggestionStatus,
};

use super::resolve_track;
use crate::output::{Output, or_blank, table};

#[derive(Debug, Subcommand)]
pub enum MetaCommand {
    /// Show a track's metadata and where each value came from.
    Show { track: String },
    /// Set a field to your own value. Your values win over the file and
    /// over every lookup. Fields: title, artist, album, album_artist,
    /// remixer, label, genre, year, bpm, key, grouping, catalog_number,
    /// isrc, release_date, track_number, disc_number, store_genre.
    Set {
        track: String,
        field: String,
        /// The new value. Leave out with --clear to blank the field.
        value: Option<String>,
        /// Blank the field instead of setting a value.
        #[arg(long, conflicts_with = "value")]
        clear: bool,
    },
    /// Drop your value or an accepted lookup, so the file's value shows again.
    Reset { track: String, field: String },
    /// List lookup suggestions waiting for review.
    Suggestions {
        /// Only this track (id or path).
        track: Option<String>,
        /// Include accepted and rejected suggestions.
        #[arg(long)]
        all: bool,
    },
    /// Accept a suggestion by id.
    Accept {
        id: i64,
        /// Allow replacing a value that is already there (yours or the file's).
        #[arg(long)]
        replace: bool,
    },
    /// Reject a suggestion by id. It won't be suggested again.
    Reject { id: i64 },
    /// Look tracks up online and add what matches as suggestions. Nothing
    /// changes until you accept them. Without tracks, looks up tracks the
    /// chosen providers haven't searched yet.
    Lookup {
        /// Tracks (ids or paths).
        tracks: Vec<String>,
        /// Providers to use: acoustid, musicbrainz, discogs (repeatable).
        /// Default: every configured one.
        #[arg(long = "provider", value_name = "NAME")]
        providers: Vec<String>,
    },
    /// Set credentials for lookups, stored in the library. AcoustID needs
    /// an application key (acoustid.org/new-application) and Discogs a
    /// personal token (discogs.com/settings/developers). Pass an empty
    /// value to remove one.
    Config {
        #[arg(long)]
        acoustid_key: Option<String>,
        #[arg(long)]
        discogs_token: Option<String>,
        /// A contact (email or URL) sent to MusicBrainz in the User-Agent,
        /// as it asks. Optional.
        #[arg(long)]
        contact: Option<String>,
    },
}

fn parse_field(name: &str) -> Result<MetadataField, CoreError> {
    MetadataField::parse(&name.to_ascii_lowercase().replace('-', "_")).ok_or_else(|| {
        CoreError::Library(LibraryError::NotFoundByName {
            kind: "metadata field",
            name: name.to_string(),
        })
    })
}

fn source_label(value: &FieldValue) -> String {
    match value.source {
        FieldSource::File => "file".into(),
        FieldSource::User => "you".into(),
        FieldSource::Lookup => format!("lookup: {}", or_blank(value.provider.as_deref())),
    }
}

pub fn run(library: &mut Library, command: &MetaCommand, out: &Output) -> Result<(), CoreError> {
    match command {
        MetaCommand::Show { track } => {
            let track = resolve_track(library, track)?;
            let values = library.field_values(track.id)?;
            out.emit(&values, || {
                println!("{}", track.path);
                let mut t = table(&["Field", "Value", "Source", "File has"]);
                for value in values
                    .iter()
                    .filter(|v| v.value.is_some() || v.source != FieldSource::File)
                {
                    t.add_row(vec![
                        value.field.to_string(),
                        or_blank(value.value.as_deref()),
                        source_label(value),
                        or_blank(value.file_value.as_deref()),
                    ]);
                }
                println!("{t}");
            });
        }
        MetaCommand::Set {
            track,
            field,
            value,
            clear,
        } => {
            let track = resolve_track(library, track)?;
            let field = parse_field(field)?;
            if value.is_none() && !clear {
                return Err(CoreError::Library(LibraryError::NotFoundByName {
                    kind: "value for",
                    name: format!("{field} (pass a value or --clear)"),
                }));
            }
            let set = library.set_user_field(track.id, field, value.as_deref())?;
            out.emit(&set, || match &set.value {
                Some(v) => println!("{field} is now {v:?} (your value)"),
                None => println!("{field} is now blank (your choice)"),
            });
        }
        MetaCommand::Reset { track, field } => {
            let track = resolve_track(library, track)?;
            let field = parse_field(field)?;
            library.clear_field_override(track.id, field)?;
            let value = library.field_value(track.id, field)?;
            out.emit(&value, || {
                println!(
                    "{field} shows the file's value again: {:?}",
                    value.value.as_deref().unwrap_or("")
                );
            });
        }
        MetaCommand::Suggestions { track, all } => {
            let track = track
                .as_deref()
                .map(|t| resolve_track(library, t))
                .transpose()?;
            let status = (!all).then_some(SuggestionStatus::Pending);
            let list = library.suggestions(track.map(|t| t.id), status)?;
            out.emit(&list, || print_suggestions(&list));
        }
        MetaCommand::Accept { id, replace } => {
            let value = library.accept_suggestion(SuggestionId(*id), *replace)?;
            out.emit(&value, || {
                println!(
                    "{} is now {:?} ({})",
                    value.field,
                    value.value.as_deref().unwrap_or(""),
                    source_label(&value)
                );
            });
        }
        MetaCommand::Lookup { tracks, providers } => lookup(library, tracks, providers, out)?,
        MetaCommand::Config {
            acoustid_key,
            discogs_token,
            contact,
        } => {
            use overtone_core::lookup::{
                LookupConfig, SETTING_ACOUSTID_KEY, SETTING_CONTACT, SETTING_DISCOGS_TOKEN,
            };
            for (key, value) in [
                (SETTING_ACOUSTID_KEY, acoustid_key),
                (SETTING_DISCOGS_TOKEN, discogs_token),
                (SETTING_CONTACT, contact),
            ] {
                if let Some(value) = value {
                    library.set_setting(key, &value.trim().to_string())?;
                }
            }
            let config = LookupConfig::load(library)?;
            let available: Vec<&str> = config.available().iter().map(|p| p.as_str()).collect();
            out.emit(&available, || {
                println!("Lookups will use: {}", available.join(", "));
            });
        }
        MetaCommand::Reject { id } => {
            library.reject_suggestion(SuggestionId(*id))?;
            let rejected = library.suggestion(SuggestionId(*id))?;
            out.emit(&rejected, || println!("Rejected suggestion {id}"));
        }
    }
    Ok(())
}

fn print_suggestions(list: &[MetadataSuggestion]) {
    if list.is_empty() {
        println!("No suggestions waiting for review.");
        return;
    }
    let mut t = table(&[
        "Id",
        "Track",
        "Field",
        "Suggested",
        "Current",
        "Provider",
        "Status",
    ]);
    for s in list {
        let current = match (&s.current.value, s.replaces_value()) {
            (Some(v), true) => format!("{v} (would replace)"),
            (value, _) => or_blank(value.as_deref()),
        };
        t.add_row(vec![
            s.id.0.to_string(),
            s.track_id.0.to_string(),
            s.field.to_string(),
            s.value.clone(),
            current,
            s.provider.clone(),
            s.status.as_str().to_string(),
        ]);
    }
    println!("{t}");
}

fn lookup(
    library: &mut Library,
    tracks: &[String],
    providers: &[String],
    out: &Output,
) -> Result<(), CoreError> {
    use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
    use overtone_core::lookup::{self, LookupConfig, LookupOptions, Provider};

    let mut chosen = Vec::new();
    for name in providers {
        chosen.push(Provider::parse(&name.to_ascii_lowercase()).ok_or_else(|| {
            CoreError::Library(LibraryError::NotFoundByName {
                kind: "lookup provider",
                name: name.clone(),
            })
        })?);
    }
    let config = LookupConfig::load(library)?;
    let effective = if chosen.is_empty() {
        config.available()
    } else {
        chosen.clone()
    };
    let ids = if tracks.is_empty() {
        lookup::tracks_needing_lookup(library, &effective)?
    } else {
        tracks
            .iter()
            .map(|t| resolve_track(library, t).map(|t| t.id))
            .collect::<Result<_, _>>()?
    };
    let bar = if out.is_json() {
        ProgressBar::hidden()
    } else {
        ProgressBar::with_draw_target(Some(ids.len() as u64), ProgressDrawTarget::stderr())
    };
    bar.set_style(
        ProgressStyle::with_template("Looking up {wide_bar} {pos}/{len} {eta}")
            .expect("valid template"),
    );
    let report = lookup::lookup_tracks(
        library,
        &ids,
        &LookupOptions {
            providers: chosen,
            cancel: None,
        },
        &|p| bar.set_position(u64::from(p.done)),
    )?;
    bar.finish_and_clear();
    out.emit(&report, || {
        let names: Vec<&str> = report.providers.iter().map(|p| p.as_str()).collect();
        println!(
            "Looked up {} track(s) with {}: {} matched, {} suggestion(s) added",
            report.looked_up,
            names.join(", "),
            report.matched,
            report.suggestions_added
        );
        for provider in &report.disabled {
            println!(
                "{} refused the credentials and was skipped; check `overtone-cli meta config`",
                provider.as_str()
            );
        }
        for failure in report.failures.iter().take(10) {
            println!(
                "  track {}: {}: {}",
                failure.track_id.0,
                failure.provider.as_str(),
                failure.message
            );
        }
        if report.suggestions_added > 0 {
            println!("Review them with `overtone-cli meta suggestions`.");
        }
    });
    Ok(())
}
