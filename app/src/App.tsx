import { useEffect } from "react";
import { HotkeysProvider } from "react-hotkeys-hook";
import { Inspector } from "./components/Inspector";
import { LibraryView } from "./components/LibraryView";
import {
  ConfirmDialog,
  ShortcutsOverlay,
  StatusBar,
  Toasts,
} from "./components/Overlays";
import { PlayerBar } from "./components/PlayerBar";
import { QuickTagPalette } from "./components/QuickTagPalette";
import { SettingsView } from "./components/SettingsView";
import { Sidebar } from "./components/Sidebar";
import { SyncView } from "./components/SyncView";
import {
  BASE_SCOPES,
  useGlobalHotkeys,
  useModalScopes,
} from "./hooks/useAppHotkeys";
import { loadLibrary, useLibrary } from "./store/library";
import { useUi, type View } from "./store/ui";
import "./styles.css";

const VIEWS: { view: View; render: () => React.ReactNode }[] = [
  { view: "library", render: () => <LibraryView tableKey="library" /> },
  { view: "review", render: () => <LibraryView tableKey="review" /> },
  { view: "sync", render: () => <SyncView /> },
  { view: "settings", render: () => <SettingsView /> },
];

function Shell() {
  useModalScopes();
  useGlobalHotkeys();
  const view = useUi((s) => s.view);
  const visited = useUi((s) => s.visited);
  const modal = useUi((s) => s.modal);
  const inspectorOpen = useUi((s) => s.inspectorOpen);
  const showInspector =
    inspectorOpen && (view === "library" || view === "review");

  return (
    <div className={`app${showInspector ? " with-inspector" : ""}`}>
      <Sidebar />
      <main className="main">
        {/* Views stay mounted once shown, so scroll and selection survive. */}
        {VIEWS.filter(({ view: v }) => visited.has(v)).map(
          ({ view: v, render }) => (
            <section
              key={v}
              className="view-slot"
              hidden={v !== view}
              aria-label={v}
            >
              {render()}
            </section>
          ),
        )}
      </main>
      {showInspector && <Inspector />}
      <PlayerBar />
      <StatusBar />
      <Toasts />
      {modal === "palette" && <QuickTagPalette />}
      {modal === "shortcuts" && <ShortcutsOverlay />}
      <ConfirmDialog />
    </div>
  );
}

export default function App() {
  const load = useLibrary((s) => s.load);
  const error = useLibrary((s) => s.error);

  useEffect(() => {
    void loadLibrary();
  }, []);

  if (load === "error") {
    return (
      <main className="fatal">
        <h1>Overtone</h1>
        <p className="error" role="alert">
          Couldn't open your library: {error}
        </p>
        <p className="dim">
          Quit and reopen Overtone after fixing the problem above.
        </p>
      </main>
    );
  }

  if (load !== "ready") {
    return (
      <main className="fatal">
        <p className="dim" role="status">
          Opening your library…
        </p>
      </main>
    );
  }

  return (
    <HotkeysProvider initiallyActiveScopes={BASE_SCOPES}>
      <Shell />
    </HotkeysProvider>
  );
}
