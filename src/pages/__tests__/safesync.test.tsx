import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import SafeSyncPage, { isLocalDestinationConnection } from "@/pages/safesync";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const CONNECTIONS = [
  {
    id: "src_1",
    name: "Staging MariaDB",
    engine: "mysql" as const,
    host: "127.0.0.1",
    port: 13306,
    database: "ministry_staging",
    username: "mdb_ro",
    useSsl: false,
  },
  {
    id: "dst_1",
    name: "Local MariaDB",
    engine: "mysql" as const,
    host: "127.0.0.1",
    port: 3306,
    database: "ministry_local",
    username: "root",
    useSsl: false,
  },
];

describe("SafeSyncPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.scrollIntoView = vi.fn();
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "connections_list":
          return CONNECTIONS;
        case "safe_sync_inspect":
          return { database: "ministry_staging", server: { engine: "mariadb", version: "10.11.19" } };
        case "security_status":
          return { enrolled: true, category: "staging", fingerprintMatch: true };
        case "safe_sync_plan":
          return { plan_fingerprint: "plan_123", engine: "mariadb", tables: [{ table: "orders", estimated_rows: 10 }] };
        case "safe_sync_validate":
          return { valid: true, warnings: [] };
        case "safe_sync_execute":
          return { run_id: "run_456", status: "completed", rowsWritten: 10 };
        case "safe_sync_status":
          return { runId: "run_456", status: "completed" };
        case "safe_sync_full_copy":
          return {
            engine: "mysql",
            sourceDatabase: "ministry_staging",
            destinationDatabase: "ministry_local",
            tablesCopied: 3,
            rowsCopied: 42,
            replaced: true,
            durationMs: 120,
            tables: [{ table: "orders", rowsCopied: 42 }],
          };
        default:
          throw new Error(`Unexpected command: ${cmd}`);
      }
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("loads connections and keeps execute disabled without plan and acknowledgement", async () => {
    render(<SafeSyncPage />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("connections_list");
    });

    const executeBtn = screen.getByRole("button", { name: /Execute Safe Sync/i });
    expect(executeBtn).toBeDisabled();

    const planBtn = screen.getByRole("button", { name: /Generate Dry-Run Plan/i });
    expect(planBtn).toBeDisabled();
  });

  it("runs inspect, plan, validate, execute, and status after selecting connections", async () => {
    render(<SafeSyncPage />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("connections_list");
    });

    fireEvent.click(screen.getByLabelText("Source connection"));
    fireEvent.click(await screen.findByText(/Staging MariaDB/i));

    fireEvent.click(screen.getByLabelText("Destination connection"));
    fireEvent.click(await screen.findByText(/Local MariaDB/i));

    fireEvent.click(screen.getByRole("button", { name: /Inspect Identities/i }));
    await waitFor(() => {
      expect(screen.getByTestId("source-identity")).toBeInTheDocument();
      expect(screen.getByTestId("destination-identity")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: /Generate Dry-Run Plan/i }));
    await waitFor(() => {
      expect(screen.getByTestId("plan-preview")).toHaveTextContent("plan_123");
    });

    fireEvent.click(screen.getByRole("button", { name: /Validate Plan/i }));
    await waitFor(() => {
      expect(screen.getByTestId("validation-preview")).toHaveTextContent('"valid": true');
    });

    const executeBtn = screen.getByRole("button", { name: /Execute Safe Sync/i });
    expect(executeBtn).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Acknowledgement Phrase (plan_fingerprint)"), {
      target: { value: "plan_123" },
    });
    expect(executeBtn).toBeEnabled();

    fireEvent.click(executeBtn);
    await waitFor(() => {
      expect(screen.getByTestId("run-report")).toHaveTextContent("run_456");
    });

    fireEvent.click(screen.getByRole("button", { name: /Refresh Run Status/i }));
    await waitFor(() => {
      expect(screen.getByTestId("run-status")).toHaveTextContent("completed");
    });
  });

  it("supports Full Copy mode with destination database confirmation", async () => {
    render(<SafeSyncPage />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("connections_list");
    });

    fireEvent.click(screen.getByLabelText("Source connection"));
    fireEvent.click(await screen.findByText(/Staging MariaDB/i));

    fireEvent.click(screen.getByLabelText("Destination connection"));
    fireEvent.click(await screen.findByText(/Local MariaDB/i));

    fireEvent.click(screen.getByRole("button", { name: /Full Copy/i }));

    const runBtn = screen.getByRole("button", { name: /Run Full Copy/i });
    expect(runBtn).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Confirm Destination Database Name"), {
      target: { value: "ministry_local" },
    });
    expect(runBtn).toBeEnabled();

    fireEvent.click(runBtn);
    await waitFor(() => {
      expect(screen.getByTestId("full-copy-report")).toHaveTextContent("ministry_local");
      expect(screen.getByTestId("full-copy-report")).toHaveTextContent('"tablesCopied": 3');
    });
  });

  it("excludes production, SSH-tunneled, and non-localhost connections from destinations", () => {
    expect(
      isLocalDestinationConnection({
        id: "p1",
        name: "Prod",
        engine: "mysql",
        host: "127.0.0.1",
        port: 3306,
        database: "prod",
        username: "dbteam",
        useSsl: false,
        target: "production",
      }),
    ).toBe(false);

    expect(
      isLocalDestinationConnection({
        id: "t1",
        name: "Tunneled",
        engine: "mysql",
        host: "127.0.0.1",
        port: 3306,
        database: "staging",
        username: "dbteam",
        useSsl: false,
        sshTunnel: { enabled: true, host: "34.101.87.166", port: 2222, username: "dbtunnel" },
      }),
    ).toBe(false);

    expect(
      isLocalDestinationConnection({
        id: "r1",
        name: "Remote",
        engine: "postgres",
        host: "10.0.0.5",
        port: 5432,
        database: "dev",
        username: "dev",
        useSsl: false,
      }),
    ).toBe(false);

    expect(
      isLocalDestinationConnection({
        id: "l1",
        name: "Local",
        engine: "postgres",
        host: "localhost",
        port: 5432,
        database: "local_dev",
        username: "postgres",
        useSsl: false,
        target: "local",
      }),
    ).toBe(true);
  });
});
