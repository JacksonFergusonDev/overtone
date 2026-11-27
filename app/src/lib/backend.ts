/**
 * The one place the UI reaches the backend, so tests can replace it with
 * `vi.mock("../lib/backend")` instead of mocking Tauri internals.
 */
import { audioSrc, commands, errorMessage, imageSrc } from "../api";

export type Backend = typeof commands;

export const backend: Backend = commands;

export { audioSrc, errorMessage, imageSrc };
