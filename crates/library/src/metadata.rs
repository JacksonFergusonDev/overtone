//! Metadata provenance: the DJ's values and accepted lookups, kept apart
//! from the file's values, plus lookup suggestions waiting for review.
//!
//! Rules, enforced here so the desktop app and CLI can't diverge:
//!
//! - A field's value is its override when one exists, else the file's.
//! - Lookups only add suggestions. They never touch Comment or Grouping,
//!   and propose `store_genre` rather than Genre.
//! - Accepting a suggestion that would replace a non-empty value (the
//!   DJ's or the file's) needs `replace = true`.
//! - Accepting one suggestion rejects the field's other pending ones; a
//!   rejected value is never suggested again by the same provider.

use overtone_model::{
    FieldSource, FieldValue, MetadataField, MetadataSuggestion, NewSuggestion, SuggestionId,
    SuggestionStatus, TrackId, TrackTags,
};
use rusqlite::{OptionalExtension, params};

use crate::{Library, LibraryError, now_millis};

struct Override {
    value: Option<String>,
    source: FieldSource,
    provider: Option<String>,
    provider_ref: Option<String>,
    set_at: i64,
}

fn parse_field(value: String) -> Result<MetadataField, LibraryError> {
    MetadataField::parse(&value).ok_or(LibraryError::Corrupt {
        column: "field",
        value,
    })
}

fn resolve(field: MetadataField, tags: &TrackTags, found: Option<Override>) -> FieldValue {
    let file_value = field.file_value(tags);
    match found {
        Some(o) => FieldValue {
            field,
            value: o.value,
            source: o.source,
            provider: o.provider,
            provider_ref: o.provider_ref,
            set_at: Some(o.set_at),
            file_value,
        },
        None => FieldValue {
            field,
            value: file_value,
            source: FieldSource::File,
            provider: None,
            provider_ref: None,
            set_at: None,
            file_value: None,
        },
    }
}

impl Library {
    fn track_tags(&self, track: TrackId) -> Result<TrackTags, LibraryError> {
        Ok(self.track(track)?.tags)
    }

    fn field_override(
        &self,
        track: TrackId,
        field: MetadataField,
    ) -> Result<Option<Override>, LibraryError> {
        self.conn
            .query_row(
                "SELECT value, source, provider, provider_ref, set_at \
                 FROM track_field_overrides WHERE track_id = ?1 AND field = ?2",
                params![track.0, field.as_str()],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?
            .map(|(value, source, provider, provider_ref, set_at)| {
                let source = FieldSource::parse(&source).ok_or(LibraryError::Corrupt {
                    column: "source",
                    value: source,
                })?;
                Ok(Override {
                    value,
                    source,
                    provider,
                    provider_ref,
                    set_at,
                })
            })
            .transpose()
    }

    /// One field's current value and source.
    pub fn field_value(
        &self,
        track: TrackId,
        field: MetadataField,
    ) -> Result<FieldValue, LibraryError> {
        let tags = self.track_tags(track)?;
        Ok(resolve(field, &tags, self.field_override(track, field)?))
    }

    /// Every field's current value and source, in [`MetadataField::ALL`] order.
    pub fn field_values(&self, track: TrackId) -> Result<Vec<FieldValue>, LibraryError> {
        let tags = self.track_tags(track)?;
        MetadataField::ALL
            .into_iter()
            .map(|field| Ok(resolve(field, &tags, self.field_override(track, field)?)))
            .collect()
    }

    /// Sets a field to a value the DJ entered (`None` or blank clears it).
    /// The DJ's value wins over the file and over every lookup. Comment is
    /// refused: it changes only through vibe mirroring.
    pub fn set_user_field(
        &self,
        track: TrackId,
        field: MetadataField,
        value: Option<&str>,
    ) -> Result<FieldValue, LibraryError> {
        if field == MetadataField::Comment {
            return Err(LibraryError::FieldNotAllowed {
                field,
                reason: "changes only through vibe mirroring",
            });
        }
        let tags = self.track_tags(track)?;
        let value = value.map(str::trim).filter(|v| !v.is_empty());
        self.conn.execute(
            "INSERT INTO track_field_overrides (track_id, field, value, source, set_at) \
             VALUES (?1, ?2, ?3, 'user', ?4) \
             ON CONFLICT (track_id, field) DO UPDATE SET value = excluded.value, \
             source = 'user', provider = NULL, provider_ref = NULL, suggestion_id = NULL, \
             set_at = excluded.set_at",
            params![track.0, field.as_str(), value, now_millis()],
        )?;
        Ok(resolve(field, &tags, self.field_override(track, field)?))
    }

    /// Drops the DJ's or an accepted lookup's value, so the file's value
    /// shows again. Returns whether there was an override.
    pub fn clear_field_override(
        &self,
        track: TrackId,
        field: MetadataField,
    ) -> Result<bool, LibraryError> {
        Ok(self.conn.execute(
            "DELETE FROM track_field_overrides WHERE track_id = ?1 AND field = ?2",
            params![track.0, field.as_str()],
        )? == 1)
    }

    /// Stores lookup suggestions as pending. Suggestions that equal the
    /// field's current value, or that the DJ already decided on, are
    /// skipped. Returns how many were added. Refuses fields lookups may not
    /// touch (Comment, Grouping, Genre).
    pub fn add_suggestions(
        &mut self,
        track: TrackId,
        suggestions: &[NewSuggestion],
    ) -> Result<usize, LibraryError> {
        if let Some(bad) = suggestions.iter().find(|s| !s.field.accepts_lookups()) {
            return Err(LibraryError::FieldNotAllowed {
                field: bad.field,
                reason: if bad.field == MetadataField::Genre {
                    "is the DJ's own; lookups suggest store_genre instead"
                } else {
                    "is the DJ's own and never filled by lookups"
                },
            });
        }
        let tags = self.track_tags(track)?;
        let now = now_millis();
        let tx = self.conn.transaction()?;
        let mut added = 0;
        for suggestion in suggestions {
            let value = suggestion.value.trim();
            if value.is_empty() {
                continue;
            }
            let current = {
                let found = tx
                    .query_row(
                        "SELECT value FROM track_field_overrides \
                         WHERE track_id = ?1 AND field = ?2",
                        params![track.0, suggestion.field.as_str()],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?;
                found.unwrap_or_else(|| suggestion.field.file_value(&tags))
            };
            if current.as_deref() == Some(value) {
                continue;
            }
            added += tx.execute(
                "INSERT OR IGNORE INTO metadata_suggestions \
                 (track_id, field, value, provider, provider_ref, confidence, fetched_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    track.0,
                    suggestion.field.as_str(),
                    value,
                    suggestion.provider,
                    suggestion.provider_ref,
                    suggestion.confidence,
                    now
                ],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }

    /// Suggestions for one track (or every track), optionally only those
    /// with `status`, newest first within each track and field.
    pub fn suggestions(
        &self,
        track: Option<TrackId>,
        status: Option<SuggestionStatus>,
    ) -> Result<Vec<MetadataSuggestion>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, track_id, field, value, provider, provider_ref, confidence, \
             fetched_at, status, decided_at FROM metadata_suggestions \
             WHERE (?1 IS NULL OR track_id = ?1) AND (?2 IS NULL OR status = ?2) \
             ORDER BY track_id, field, fetched_at DESC, id",
        )?;
        let rows = stmt
            .query_map(
                params![track.map(|t| t.0), status.map(SuggestionStatus::as_str)],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<f64>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(rows.len());
        let mut tags_cache: Option<(i64, TrackTags)> = None;
        for (id, track_id, field, value, provider, provider_ref, confidence, fetched, st, at) in
            rows
        {
            let field = parse_field(field)?;
            let status = SuggestionStatus::parse(&st).ok_or(LibraryError::Corrupt {
                column: "status",
                value: st,
            })?;
            let tags = match &tags_cache {
                Some((cached, tags)) if *cached == track_id => tags.clone(),
                _ => {
                    let tags = self.track_tags(TrackId(track_id))?;
                    tags_cache = Some((track_id, tags.clone()));
                    tags
                }
            };
            let current = resolve(field, &tags, self.field_override(TrackId(track_id), field)?);
            out.push(MetadataSuggestion {
                id: SuggestionId(id),
                track_id: TrackId(track_id),
                field,
                value,
                provider,
                provider_ref,
                confidence,
                fetched_at: fetched,
                status,
                decided_at: at,
                current,
            });
        }
        Ok(out)
    }

    pub fn suggestion(&self, id: SuggestionId) -> Result<MetadataSuggestion, LibraryError> {
        let track: i64 = self
            .conn
            .query_row(
                "SELECT track_id FROM metadata_suggestions WHERE id = ?1",
                [id.0],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(LibraryError::NotFound {
                kind: "suggestion",
                id: id.0,
            })?;
        self.suggestions(Some(TrackId(track)), None)?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or(LibraryError::NotFound {
                kind: "suggestion",
                id: id.0,
            })
    }

    /// Accepts a pending suggestion: the value becomes the field's value
    /// with source `lookup`, and the field's other pending suggestions are
    /// rejected. When the field already holds a different non-empty value
    /// (the DJ's or the file's), `replace` must be true.
    pub fn accept_suggestion(
        &mut self,
        id: SuggestionId,
        replace: bool,
    ) -> Result<FieldValue, LibraryError> {
        let suggestion = self.suggestion(id)?;
        if suggestion.status != SuggestionStatus::Pending {
            return Err(LibraryError::SuggestionDecided {
                id: id.0,
                status: suggestion.status.as_str(),
            });
        }
        if suggestion.replaces_value() && !replace {
            return Err(LibraryError::WouldReplace {
                field: suggestion.field,
                current: suggestion.current.value.clone().unwrap_or_default(),
            });
        }
        let now = now_millis();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO track_field_overrides \
             (track_id, field, value, source, provider, provider_ref, suggestion_id, set_at) \
             VALUES (?1, ?2, ?3, 'lookup', ?4, ?5, ?6, ?7) \
             ON CONFLICT (track_id, field) DO UPDATE SET value = excluded.value, \
             source = 'lookup', provider = excluded.provider, \
             provider_ref = excluded.provider_ref, suggestion_id = excluded.suggestion_id, \
             set_at = excluded.set_at",
            params![
                suggestion.track_id.0,
                suggestion.field.as_str(),
                suggestion.value,
                suggestion.provider,
                suggestion.provider_ref,
                id.0,
                now
            ],
        )?;
        tx.execute(
            "UPDATE metadata_suggestions SET status = 'accepted', decided_at = ?2 WHERE id = ?1",
            params![id.0, now],
        )?;
        tx.execute(
            "UPDATE metadata_suggestions SET status = 'rejected', decided_at = ?3 \
             WHERE track_id = ?1 AND field = ?2 AND status = 'pending'",
            params![suggestion.track_id.0, suggestion.field.as_str(), now],
        )?;
        tx.commit()?;
        self.field_value(suggestion.track_id, suggestion.field)
    }

    /// Rejects a pending suggestion. The same provider won't suggest the
    /// same value for this field again.
    pub fn reject_suggestion(&self, id: SuggestionId) -> Result<(), LibraryError> {
        let suggestion = self.suggestion(id)?;
        if suggestion.status != SuggestionStatus::Pending {
            return Err(LibraryError::SuggestionDecided {
                id: id.0,
                status: suggestion.status.as_str(),
            });
        }
        self.conn.execute(
            "UPDATE metadata_suggestions SET status = 'rejected', decided_at = ?2 WHERE id = ?1",
            params![id.0, now_millis()],
        )?;
        Ok(())
    }

    /// Records that `track` was looked up with `provider`.
    pub fn record_lookup(
        &self,
        track: TrackId,
        provider: &str,
        matched: bool,
    ) -> Result<(), LibraryError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO lookup_runs (track_id, provider, looked_up_at, matched) \
             VALUES (?1, ?2, ?3, ?4)",
            params![track.0, provider, now_millis(), i64::from(matched)],
        )?;
        Ok(())
    }

    /// Present tracks never looked up with `provider`.
    pub fn tracks_needing_lookup(&self, provider: &str) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tracks t \
             LEFT JOIN lookup_runs l ON l.track_id = t.id AND l.provider = ?1 \
             WHERE t.missing_since IS NULL AND l.track_id IS NULL ORDER BY t.path",
        )?;
        let ids = stmt
            .query_map([provider], |row| row.get::<_, i64>(0))?
            .map(|id| id.map(TrackId))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Tracks with at least one pending suggestion.
    pub fn tracks_with_pending_suggestions(&self) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT track_id FROM metadata_suggestions \
             WHERE status = 'pending' ORDER BY track_id",
        )?;
        let ids = stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .map(|id| id.map(TrackId))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Tracks with at least one field whose value has `source`.
    pub fn tracks_with_field_source(
        &self,
        source: FieldSource,
    ) -> Result<Vec<TrackId>, LibraryError> {
        if source == FieldSource::File {
            let mut stmt = self.conn.prepare("SELECT id FROM tracks ORDER BY id")?;
            return Ok(stmt
                .query_map([], |row| row.get::<_, i64>(0))?
                .map(|id| id.map(TrackId))
                .collect::<Result<Vec<_>, _>>()?);
        }
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT track_id FROM track_field_overrides WHERE source = ?1 \
             ORDER BY track_id",
        )?;
        let ids = stmt
            .query_map([source.as_str()], |row| row.get::<_, i64>(0))?
            .map(|id| id.map(TrackId))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScannedTrack;
    use overtone_model::{AudioProperties, TrackTags};

    fn library_with_track(tags: TrackTags) -> (Library, TrackId) {
        let mut lib = Library::open_in_memory().expect("open");
        let scanned = ScannedTrack {
            path: "/music/a.flac".into(),
            file_size: 1,
            modified_at: 1,
            audio_hash: None,
            properties: AudioProperties::default(),
            tags,
        };
        lib.upsert_scanned(&scanned).expect("insert");
        let id = lib
            .track_by_path("/music/a.flac")
            .expect("query")
            .expect("track")
            .id;
        (lib, id)
    }

    fn suggestion(field: MetadataField, value: &str) -> NewSuggestion {
        NewSuggestion {
            field,
            value: value.into(),
            provider: "musicbrainz".into(),
            provider_ref: Some("mbid-1".into()),
            confidence: Some(0.9),
        }
    }

    #[test]
    fn file_values_have_file_source_until_overridden() {
        let (lib, id) = library_with_track(TrackTags {
            label: Some("Hospital".into()),
            ..TrackTags::default()
        });
        let label = lib.field_value(id, MetadataField::Label).expect("value");
        assert_eq!(label.source, FieldSource::File);
        assert_eq!(label.value.as_deref(), Some("Hospital"));

        let set = lib
            .set_user_field(id, MetadataField::Label, Some(" Hospital Records "))
            .expect("set");
        assert_eq!(set.source, FieldSource::User);
        assert_eq!(set.value.as_deref(), Some("Hospital Records"));
        assert_eq!(set.file_value.as_deref(), Some("Hospital"));

        assert!(
            lib.clear_field_override(id, MetadataField::Label)
                .expect("clear")
        );
        let back = lib.field_value(id, MetadataField::Label).expect("value");
        assert_eq!(back.source, FieldSource::File);
        assert_eq!(back.value.as_deref(), Some("Hospital"));
    }

    #[test]
    fn comment_is_never_a_user_override() {
        let (lib, id) = library_with_track(TrackTags::default());
        assert!(matches!(
            lib.set_user_field(id, MetadataField::Comment, Some("x")),
            Err(LibraryError::FieldNotAllowed { .. })
        ));
    }

    #[test]
    fn lookups_cannot_touch_personal_fields_or_genre() {
        let (mut lib, id) = library_with_track(TrackTags::default());
        for field in [
            MetadataField::Comment,
            MetadataField::Grouping,
            MetadataField::Genre,
        ] {
            assert!(matches!(
                lib.add_suggestions(id, &[suggestion(field, "x")]),
                Err(LibraryError::FieldNotAllowed { .. })
            ));
        }
        assert_eq!(
            lib.add_suggestions(id, &[suggestion(MetadataField::StoreGenre, "Drum & Bass")])
                .expect("store genre"),
            1
        );
    }

    #[test]
    fn suggestions_are_pending_until_accepted() {
        let (mut lib, id) = library_with_track(TrackTags::default());
        lib.add_suggestions(id, &[suggestion(MetadataField::Label, "Hospital")])
            .expect("add");
        let value = lib.field_value(id, MetadataField::Label).expect("value");
        assert_eq!(value.value, None, "a suggestion is not the track's value");

        let pending = lib
            .suggestions(Some(id), Some(SuggestionStatus::Pending))
            .expect("list");
        assert_eq!(pending.len(), 1);
        assert!(!pending[0].replaces_value());
        assert_eq!(lib.tracks_with_pending_suggestions().expect("ids"), [id]);

        let accepted = lib.accept_suggestion(pending[0].id, false).expect("accept");
        assert_eq!(accepted.source, FieldSource::Lookup);
        assert_eq!(accepted.provider.as_deref(), Some("musicbrainz"));
        assert_eq!(accepted.provider_ref.as_deref(), Some("mbid-1"));
        assert_eq!(accepted.value.as_deref(), Some("Hospital"));
        assert!(
            lib.tracks_with_pending_suggestions()
                .expect("ids")
                .is_empty()
        );
        assert_eq!(
            lib.tracks_with_field_source(FieldSource::Lookup)
                .expect("ids"),
            [id]
        );
    }

    #[test]
    fn replacing_a_value_needs_an_explicit_choice() {
        let (mut lib, id) = library_with_track(TrackTags {
            album: Some("From the file".into()),
            ..TrackTags::default()
        });
        lib.set_user_field(id, MetadataField::Label, Some("Mine"))
            .expect("set");
        lib.add_suggestions(
            id,
            &[
                suggestion(MetadataField::Label, "Theirs"),
                suggestion(MetadataField::Album, "Fetched album"),
            ],
        )
        .expect("add");
        for s in lib.suggestions(Some(id), None).expect("list") {
            assert!(s.replaces_value());
            assert!(matches!(
                lib.accept_suggestion(s.id, false),
                Err(LibraryError::WouldReplace { .. })
            ));
        }
        let label = lib.field_value(id, MetadataField::Label).expect("value");
        assert_eq!(label.source, FieldSource::User);
        assert_eq!(label.value.as_deref(), Some("Mine"));

        let album = lib
            .suggestions(Some(id), None)
            .expect("list")
            .into_iter()
            .find(|s| s.field == MetadataField::Album)
            .expect("album");
        let replaced = lib.accept_suggestion(album.id, true).expect("replace");
        assert_eq!(replaced.value.as_deref(), Some("Fetched album"));
        assert_eq!(replaced.file_value.as_deref(), Some("From the file"));
    }

    #[test]
    fn accepting_one_rejects_the_rest_and_rejections_stick() {
        let (mut lib, id) = library_with_track(TrackTags::default());
        lib.add_suggestions(
            id,
            &[
                suggestion(MetadataField::Label, "A"),
                suggestion(MetadataField::Label, "B"),
            ],
        )
        .expect("add");
        let all = lib.suggestions(Some(id), None).expect("list");
        lib.accept_suggestion(all[0].id, false).expect("accept");
        let after = lib.suggestions(Some(id), None).expect("list");
        assert_eq!(
            after
                .iter()
                .filter(|s| s.status == SuggestionStatus::Rejected)
                .count(),
            1
        );
        assert!(matches!(
            lib.accept_suggestion(all[0].id, false),
            Err(LibraryError::SuggestionDecided { .. })
        ));
        // The same provider re-suggesting a decided value adds nothing.
        let again = lib
            .add_suggestions(
                id,
                &[
                    suggestion(MetadataField::Label, "A"),
                    suggestion(MetadataField::Label, "B"),
                ],
            )
            .expect("re-add");
        assert_eq!(again, 0);
    }

    #[test]
    fn remembers_which_tracks_were_looked_up() {
        let (lib, id) = library_with_track(TrackTags::default());
        assert_eq!(lib.tracks_needing_lookup("musicbrainz").expect("ids"), [id]);
        lib.record_lookup(id, "musicbrainz", false).expect("record");
        assert!(
            lib.tracks_needing_lookup("musicbrainz")
                .expect("ids")
                .is_empty()
        );
        assert_eq!(lib.tracks_needing_lookup("discogs").expect("ids"), [id]);
    }

    #[test]
    fn rescans_keep_overrides() {
        let (mut lib, id) = library_with_track(TrackTags {
            title: Some("Old".into()),
            ..TrackTags::default()
        });
        lib.set_user_field(id, MetadataField::Title, Some("Mine"))
            .expect("set");
        let rescanned = ScannedTrack {
            path: "/music/a.flac".into(),
            file_size: 2,
            modified_at: 2,
            audio_hash: None,
            properties: AudioProperties::default(),
            tags: TrackTags {
                title: Some("New in file".into()),
                ..TrackTags::default()
            },
        };
        lib.upsert_scanned(&rescanned).expect("rescan");
        let title = lib.field_value(id, MetadataField::Title).expect("value");
        assert_eq!(title.value.as_deref(), Some("Mine"));
        assert_eq!(title.file_value.as_deref(), Some("New in file"));
    }
}
