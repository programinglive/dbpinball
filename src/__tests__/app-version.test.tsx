import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import pkg from "../../package.json";

// The shell calls Tauri commands on mount; the version assertion does not need
// them, so the bridge is stubbed.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockRejectedValue(new Error("not running under Tauri")),
}));

import App from "../App";

describe("App shell", () => {
  it("shows the packaged version, not a hardcoded one", () => {
    render(<App />);
    expect(screen.getByText(`Version ${pkg.version}`)).toBeInTheDocument();
    expect(screen.queryByText(/0\.1\.4/)).not.toBeInTheDocument();
  });
});
