import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "../App";
import { TableRectContext } from "../components/TrackTable";
import { resetJobs } from "../store/jobs";
import { resetLibrary } from "../store/library";
import { resetPlayer } from "../store/player";
import { useToasts } from "../store/toasts";
import { resetUi } from "../store/ui";
import { useConfirm } from "../store/confirm";

/** A viewport tall enough for every fixture row. */
export const TEST_RECT = { width: 1200, height: 600 };

export function resetStores(): void {
  resetLibrary();
  resetUi();
  resetPlayer();
  resetJobs();
  useToasts.setState({ toasts: [] });
  useConfirm.setState({ request: null, resolve: null });
}

/** Renders the whole app against the mocked backend and waits for the library. */
export async function renderApp() {
  const user = userEvent.setup();
  const result = render(
    <TableRectContext.Provider value={TEST_RECT}>
      <App />
    </TableRectContext.Provider>,
  );
  await screen.findByRole("navigation", { name: "Main" });
  return { user, ...result };
}
