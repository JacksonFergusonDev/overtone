/** Every keyboard shortcut, for the `?` overlay. Keep docs/desktop-app.md in step. */
export interface ShortcutGroup {
  title: string;
  shortcuts: { keys: string[]; action: string }[];
}

export const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    title: "Playback",
    shortcuts: [
      { keys: ["Space"], action: "Play or pause" },
      { keys: ["J"], action: "Next track in the table" },
      { keys: ["K"], action: "Previous track in the table" },
      { keys: ["←", "→"], action: "Seek back or forward 5 seconds" },
      {
        keys: ["Shift ←", "Shift →"],
        action: "Seek back or forward 30 seconds",
      },
      { keys: ["1 to 9"], action: "Jump to 10% to 90% of the track" },
    ],
  },
  {
    title: "Track table",
    shortcuts: [
      { keys: ["↑", "↓"], action: "Move the cursor" },
      { keys: ["Shift ↑", "Shift ↓"], action: "Extend the selection" },
      { keys: ["Home", "End"], action: "First or last track" },
      { keys: ["Page Up", "Page Down"], action: "Move by a page" },
      { keys: ["Enter"], action: "Play the focused track" },
      { keys: ["⌘ A"], action: "Select every track shown" },
      { keys: ["Esc"], action: "Clear the selection" },
      { keys: ["/", "⌘ F"], action: "Search the table" },
    ],
  },
  {
    title: "Tagging",
    shortcuts: [
      {
        keys: ["T", "⌘ K"],
        action: "Open the quick-tag palette for the selection",
      },
      {
        keys: ["Enter"],
        action: "Toggle the highlighted vibe (in the palette)",
      },
      { keys: ["⌥ 1 to ⌥ 9"], action: "Toggle a recent vibe (in the palette)" },
    ],
  },
  {
    title: "Quality review",
    shortcuts: [
      { keys: ["I"], action: "Show or hide the quality inspector" },
      { keys: ["O"], action: "Mark Verify OK" },
      { keys: ["R"], action: "Mark Replace wanted" },
      { keys: ["U"], action: "Clear the review" },
      { keys: ["L"], action: "Switch the spectrogram between linear and log" },
    ],
  },
  {
    title: "Views",
    shortcuts: [
      { keys: ["⌘ 1"], action: "Library" },
      { keys: ["⌘ 2"], action: "Needs review" },
      { keys: ["⌘ 3"], action: "Sync" },
      { keys: ["⌘ 4"], action: "Settings" },
      { keys: ["?"], action: "Show this list" },
    ],
  },
];
