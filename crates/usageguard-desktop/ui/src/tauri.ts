const tauri = (window as any).__TAURI__;

export const invoke = tauri?.core?.invoke as
  | ((cmd: string, args?: Record<string, unknown>) => Promise<any>)
  | undefined;

export const listen = tauri?.event?.listen as
  | ((event: string, handler: (event: unknown) => void) => Promise<() => void>)
  | undefined;

export const currentWindow = tauri?.window?.getCurrentWindow?.() ?? tauri?.window?.getCurrent?.() ?? null;
