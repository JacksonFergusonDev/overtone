import { useEffect } from "react";
import { useHotkeys, useHotkeysContext } from "react-hotkeys-hook";
import { digitFraction } from "../lib/waveform";
import { openPalette } from "../store/actions";
import { seekBy, seekFraction, skip, togglePlay } from "../store/player";
import {
  openModal,
  setView,
  toggleInspector,
  useUi,
  type Modal,
} from "../store/ui";

/** Scopes active when no modal is open. */
export const BASE_SCOPES = ["global", "table", "inspector"];

const MODAL_SCOPES: Record<Exclude<Modal, null>, string> = {
  palette: "palette",
  shortcuts: "modal",
  confirm: "modal",
};

/** Turns off the app's shortcuts while a modal (palette, dialog) is open. */
export function useModalScopes(): void {
  const modal = useUi((s) => s.modal);
  const { enableScope, disableScope } = useHotkeysContext();
  useEffect(() => {
    if (modal === null) {
      BASE_SCOPES.forEach(enableScope);
      return;
    }
    BASE_SCOPES.forEach(disableScope);
    enableScope(MODAL_SCOPES[modal]);
    return () => disableScope(MODAL_SCOPES[modal]);
  }, [modal, enableScope, disableScope]);
}

const global = { scopes: "global", preventDefault: true } as const;

/** App-wide shortcuts. They never fire while you type in a field. */
export function useGlobalHotkeys(): void {
  useHotkeys("space", () => togglePlay(), global);
  useHotkeys("j", () => skip(1), global);
  useHotkeys("k", () => skip(-1), global);
  useHotkeys(
    ["left", "shift+left"],
    (event) => seekBy(event.shiftKey ? -30 : -5),
    global,
  );
  useHotkeys(
    ["right", "shift+right"],
    (event) => seekBy(event.shiftKey ? 30 : 5),
    global,
  );
  useHotkeys(
    ["1", "2", "3", "4", "5", "6", "7", "8", "9"],
    (event) => {
      const fraction = digitFraction(event.code.replace("Digit", ""));
      if (fraction !== null) seekFraction(fraction);
    },
    global,
  );
  useHotkeys("t", () => openPalette(), global);
  // Cmd+K also works from the search field.
  useHotkeys("mod+k", () => openPalette(), {
    ...global,
    enableOnFormTags: true,
  });
  useHotkeys("i", () => toggleInspector(), global);
  useHotkeys("shift+slash", () => openModal("shortcuts"), global);
  useHotkeys("mod+1", () => setView("library"), global);
  useHotkeys("mod+2", () => setView("review"), global);
  useHotkeys("mod+3", () => setView("sync"), global);
  useHotkeys("mod+4", () => setView("settings"), global);
}
