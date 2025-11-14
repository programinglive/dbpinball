import { useCallback } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function useTauriWindowControls() {
  const handleMinimizeWindow = useCallback(async () => {
    try {
      const window = getCurrentWindow();
      await window.minimize();
    } catch (e) {
      console.error("Minimize failed:", e);
    }
  }, []);

  const handleToggleMaximize = useCallback(async () => {
    try {
      const window = getCurrentWindow();
      const maximized = await window.isMaximized();
      if (maximized) {
        await window.unmaximize();
      } else {
        await window.maximize();
      }
    } catch (e) {
      console.error("Toggle maximize failed:", e);
    }
  }, []);

  const handleCloseWindow = useCallback(async () => {
    try {
      const window = getCurrentWindow();
      await window.close();
    } catch (e) {
      console.error("Close failed:", e);
    }
  }, []);

  return {
    handleMinimizeWindow,
    handleToggleMaximize,
    handleCloseWindow,
  };
}
