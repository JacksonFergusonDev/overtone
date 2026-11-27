import {
  Pause,
  Play,
  SkipBack,
  SkipForward,
  Volume2,
  VolumeX,
} from "lucide-react";
import { useCallback } from "react";
import { formatClock, trackTitle } from "../lib/format";
import { useLibrary } from "../store/library";
import {
  registerAudio,
  setVolume,
  skip,
  togglePlay,
  usePlayer,
} from "../store/player";
import { Waveform } from "./Waveform";

/** The single `<audio>` element, transport controls, and waveform scrubber. */
export function PlayerBar() {
  const currentId = usePlayer((s) => s.currentId);
  const playing = usePlayer((s) => s.playing);
  const position = usePlayer((s) => s.position);
  const duration = usePlayer((s) => s.duration);
  const volume = usePlayer((s) => s.volume);
  const error = usePlayer((s) => s.error);
  const track = useLibrary((s) =>
    currentId === null ? undefined : s.tracks.get(currentId),
  );

  const audioRef = useCallback((element: HTMLAudioElement | null) => {
    registerAudio(element);
  }, []);

  return (
    <footer className="player" aria-label="Player">
      <audio
        ref={audioRef}
        preload="auto"
        onTimeUpdate={(event) =>
          usePlayer.setState({ position: event.currentTarget.currentTime })
        }
        onLoadedMetadata={(event) => {
          const length = event.currentTarget.duration;
          if (Number.isFinite(length)) usePlayer.setState({ duration: length });
        }}
        onPlay={() => usePlayer.setState({ playing: true })}
        onPause={() => usePlayer.setState({ playing: false })}
        onEnded={() => usePlayer.setState({ playing: false })}
        onError={() =>
          usePlayer.setState({
            playing: false,
            error:
              "This file can't be played. The format may be unsupported or the file unreadable.",
          })
        }
      />
      <div className="transport">
        <button
          type="button"
          className="icon-button"
          aria-label="Previous track (K)"
          onClick={() => skip(-1)}
        >
          <SkipBack size={16} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="icon-button play-button"
          aria-label={playing ? "Pause (Space)" : "Play (Space)"}
          onClick={togglePlay}
        >
          {playing ? (
            <Pause size={18} aria-hidden="true" />
          ) : (
            <Play size={18} aria-hidden="true" />
          )}
        </button>
        <button
          type="button"
          className="icon-button"
          aria-label="Next track (J)"
          onClick={() => skip(1)}
        >
          <SkipForward size={16} aria-hidden="true" />
        </button>
      </div>
      <div className="now-playing">
        {track ? (
          <>
            <span className="np-title truncate" title={trackTitle(track)}>
              {trackTitle(track)}
            </span>
            <span
              className="np-artist truncate"
              title={track.tags.artist ?? ""}
            >
              {error ? (
                <span className="error-text">{error}</span>
              ) : (
                (track.tags.artist ?? "")
              )}
            </span>
          </>
        ) : (
          <span className="dim">Nothing playing</span>
        )}
      </div>
      <span className="clock num" aria-label="Position">
        {formatClock(position)}
      </span>
      <Waveform trackId={currentId} />
      <span className="clock num" aria-label="Duration">
        {formatClock(duration)}
      </span>
      <div className="volume">
        <button
          type="button"
          className="icon-button"
          aria-label={volume === 0 ? "Unmute" : "Mute"}
          onClick={() => setVolume(volume === 0 ? 0.8 : 0)}
        >
          {volume === 0 ? (
            <VolumeX size={16} aria-hidden="true" />
          ) : (
            <Volume2 size={16} aria-hidden="true" />
          )}
        </button>
        <input
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={volume}
          aria-label="Volume"
          onChange={(event) => setVolume(Number(event.target.value))}
        />
      </div>
    </footer>
  );
}
