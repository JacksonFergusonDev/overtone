import { create } from "zustand";
import { closeModal, openModal } from "./ui";

export interface ConfirmRequest {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
}

interface ConfirmState {
  request: ConfirmRequest | null;
  resolve: ((ok: boolean) => void) | null;
}

export const useConfirm = create<ConfirmState>(() => ({
  request: null,
  resolve: null,
}));

/** Asks the DJ to confirm in a modal dialog. Resolves false on cancel. */
export function confirmAction(request: ConfirmRequest): Promise<boolean> {
  useConfirm.getState().resolve?.(false);
  return new Promise((resolve) => {
    useConfirm.setState({ request, resolve });
    openModal("confirm");
  });
}

export function settleConfirm(ok: boolean): void {
  const { resolve } = useConfirm.getState();
  useConfirm.setState({ request: null, resolve: null });
  closeModal();
  resolve?.(ok);
}
