import { create } from "zustand";

export type ToastKind = "info" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

interface ToastState {
  toasts: Toast[];
}

export const useToasts = create<ToastState>(() => ({ toasts: [] }));

let nextId = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

export function dismissToast(id: number): void {
  const timer = timers.get(id);
  if (timer) clearTimeout(timer);
  timers.delete(id);
  useToasts.setState((state) => ({
    toasts: state.toasts.filter((toast) => toast.id !== id),
  }));
}

/** Shows a message; info clears after 4 s, errors after 10 s. */
export function showToast(kind: ToastKind, message: string): number {
  const id = nextId;
  nextId += 1;
  useToasts.setState((state) => ({
    toasts: [...state.toasts.slice(-4), { id, kind, message }],
  }));
  timers.set(
    id,
    setTimeout(() => dismissToast(id), kind === "error" ? 10_000 : 4_000),
  );
  return id;
}

export const notifyError = (message: string): number =>
  showToast("error", message);
export const notify = (message: string): number => showToast("info", message);
