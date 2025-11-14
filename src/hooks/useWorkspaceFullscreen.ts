import { useEffect } from "react";
import type { WorkspaceState } from "@/pages/connections.types";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function useWorkspaceFullscreen(workspace: WorkspaceState) {
  useEffect(() => {
    if (!workspace.open) {
      return;
    }

    const enterFullscreen = async () => {
      try {
        const window = getCurrentWindow();
        await window.setFullscreen(true);
      } catch (e) {
        console.error("Enter fullscreen failed:", e);
      }
    };

    void enterFullscreen();

    return () => {
      const exitFullscreen = async () => {
        try {
          const window = getCurrentWindow();
          await window.setFullscreen(false);
        } catch (e) {
          console.error("Exit fullscreen failed:", e);
        }
      };

      void exitFullscreen();
    };
  }, [workspace.open]);
}
