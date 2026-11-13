import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import ConnectionsPage from "@/pages/connections";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const setFullscreenMock = vi.fn((_: boolean) => Promise.resolve());

vi.mock("@tauri-apps/api/window", () => {
  const minimizeMock = vi.fn(() => Promise.resolve());
  const maximizeMock = vi.fn(() => Promise.resolve());
  const unmaximizeMock = vi.fn(() => Promise.resolve());
  const isMaximizedMock = vi.fn(() => Promise.resolve(false));
  const closeMock = vi.fn(() => Promise.resolve());

  return {
    getCurrentWindow: vi.fn(() => ({
      setFullscreen: setFullscreenMock,
      minimize: minimizeMock,
      maximize: maximizeMock,
      unmaximize: unmaximizeMock,
      isMaximized: isMaximizedMock,
      close: closeMock,
    })),
  };
});

const createConnections = () => [
  {
    id: "conn_1",
    name: "Analytics",
    engine: "postgres" as const,
    host: "analytics.example.com",
    port: 5432,
    database: "analytics",
    username: "analyst",
    useSsl: true,
  },
  {
    id: "conn_2",
    name: "Staging",
    engine: "mysql" as const,
    host: "staging-db.internal",
    port: 3306,
    database: "staging",
    username: "staging",
    useSsl: false,
  },
];

type Overrides = Record<string, (payload: unknown) => unknown>;

/**
 * A stateful mock of the shared connection store.
 *
 * The page no longer reads localStorage as its source of truth: it calls
 * `connections_list` and mutates through `connections_create` / `_update` /
 * `_delete`. The mock keeps a mutable list so a delete is visible on reload.
 */
function mockStore(initial: ReturnType<typeof createConnections>, overrides: Overrides = {}) {
  let current = [...initial];
  const invokeMock = vi.mocked(invoke);

  invokeMock.mockImplementation(async (command: string, payload: unknown) => {
    if (overrides[command]) {
      return overrides[command](payload);
    }

    if (command === "connections_list") {
      return [...current];
    }
    if (command === "connections_migration_status") {
      return { migratedFrom: "localStorage@v1", revision: 1, profiles: current.length };
    }
    if (command === "connections_delete") {
      const id = (payload as { id: string }).id;
      current = current.filter((connection) => connection.id !== id);
      return true;
    }
    if (command === "connections_create") {
      const created = { ...(payload as { connection: object }).connection, id: "conn_new" };
      current.push(created as (typeof current)[number]);
      return created;
    }
    if (command === "connections_update") {
      const connection = (payload as { connection: { id: string } }).connection;
      current = current.map((existing) => (existing.id === connection.id ? (connection as (typeof current)[number]) : existing));
      return undefined;
    }
    if (command === "test_conn") {
      return;
    }
    if (command === "list_databases_cmd") {
      return ["main", "reporting"];
    }

    return undefined;
  });

  return invokeMock;
}

describe("ConnectionsPage", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    vi.mocked(invoke).mockReset();
    setFullscreenMock.mockClear();
  });

  afterEach(() => {
    localStorage.clear();
    cleanup();
  });

  it("shows empty state when no connections exist", async () => {
    mockStore([]);
    render(<ConnectionsPage />);

    expect(await screen.findByText("No connections yet.")).toBeInTheDocument();
    expect(
      await screen.findByText("Create a connection to get started.")
    ).toBeInTheDocument();
  });

  it("opens workspace overlay and lists databases after connecting", async () => {
    const connections = createConnections();
    const invokeMock = mockStore(connections);

    render(<ConnectionsPage />);

    const analyticsTitle = await screen.findByText("Analytics");
    const connectionTitles = Array.from(
      document.querySelectorAll<HTMLElement>(".connection-row__title")
    ).map((el) => el.textContent?.trim());

    expect(connectionTitles).toEqual(["Analytics", "Staging"]);

    const analyticsCard = analyticsTitle.closest<HTMLElement>(".connection-row");
    expect(analyticsCard).toBeTruthy();

    await act(async () => {
      fireEvent.doubleClick(analyticsCard!);
    });

    expect(invokeMock).toHaveBeenCalledWith("test_conn", { conn: connections[0] });
    expect(invokeMock).toHaveBeenCalledWith("list_databases_cmd", { conn: connections[0] });

    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    await waitFor(() => {
      expect(setFullscreenMock).toHaveBeenCalledWith(true);
    });

    const databasePickerDialog = await screen.findByRole("dialog", { name: "Open database" });
    expect(databasePickerDialog).toBeInTheDocument();
    expect(within(databasePickerDialog).getByText("main")).toBeInTheDocument();

    const closeDatabasePickerButton = within(databasePickerDialog).getByRole("button", {
      name: "Close database picker",
    });
    fireEvent.click(closeDatabasePickerButton);

    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Open database" })).not.toBeInTheDocument();
    });

    const [openListButton] = await screen.findAllByRole("button", { name: "Open connection list" });
    expect(openListButton).toBeInTheDocument();
    expect(screen.getByLabelText("Settings")).toBeInTheDocument();

    expect(screen.queryByLabelText("Refresh metadata")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Run query")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Search")).not.toBeInTheDocument();
    expect(screen.queryByText("Menu")).not.toBeInTheDocument();

    expect(await screen.findByText("Items content coming soon.")).toBeInTheDocument();
    expect(
      screen.getByText("Select a database from the left to explore its tables.")
    ).toBeInTheDocument();
  });

  it("connects via context menu actions", async () => {
    const connections = createConnections();
    const invokeMock = mockStore(connections);

    render(<ConnectionsPage />);

    const analyticsCard = await screen.findByText("Analytics");
    fireEvent.contextMenu(analyticsCard, { clientX: 160, clientY: 200 });

    const connectButton = await screen.findByRole("button", { name: "Connect" });

    await act(async () => {
      fireEvent.mouseDown(connectButton);
    });

    expect(invokeMock).toHaveBeenCalledWith("test_conn", { conn: connections[0] });
    expect(invokeMock).toHaveBeenCalledWith("list_databases_cmd", { conn: connections[0] });
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("opens edit dialog from context menu", async () => {
    const connections = createConnections();
    const invokeMock = mockStore(connections);

    render(<ConnectionsPage />);

    const analyticsCard = await screen.findByText("Analytics");
    fireEvent.contextMenu(analyticsCard, { clientX: 120, clientY: 120 });

    const editButton = await screen.findByRole("button", { name: /Edit/ });

    await act(async () => {
      fireEvent.mouseDown(editButton);
    });

    expect(await screen.findByText("Edit Database Connection")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Analytics")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(invokeMock).not.toHaveBeenCalledWith("connections_update", expect.anything());
    expect(invokeMock).not.toHaveBeenCalledWith("connections_create", expect.anything());
  });

  it("deletes connection through the shared store when confirmed", async () => {
    const connections = createConnections();
    const invokeMock = mockStore(connections);

    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<ConnectionsPage />);

    const analyticsCard = await screen.findByText("Analytics");
    fireEvent.contextMenu(analyticsCard, { clientX: 80, clientY: 80 });

    const deleteButton = await screen.findByRole("button", { name: "Delete" });

    await act(async () => {
      fireEvent.mouseDown(deleteButton);
    });

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("connections_delete", {
      id: "conn_1",
      expectedRevision: 1,
    });

    await waitFor(() => {
      expect(screen.queryByText("Analytics")).not.toBeInTheDocument();
    });

    confirmSpy.mockRestore();
  });

  it("shows friendly error messaging when connection test fails", async () => {
    const connections = createConnections();
    const invokeMock = mockStore(connections, {
      test_conn: () => {
        throw new Error("No such host is known");
      },
    });

    render(<ConnectionsPage />);

    const analyticsCard = await screen.findByText("Analytics");

    await act(async () => {
      fireEvent.doubleClick(analyticsCard);
    });

    expect(invokeMock).toHaveBeenCalledWith("test_conn", { conn: connections[0] });
    expect(invokeMock).not.toHaveBeenCalledWith("list_databases_cmd", expect.anything());

    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    const errorMessages = await screen.findAllByText((content) =>
      content.includes("Unable to resolve host. Details: No such host is known")
    );
    expect(errorMessages.length).toBeGreaterThan(0);
    const failureChips = await screen.findAllByText("Failed");
    expect(failureChips.length).toBeGreaterThan(0);

    await waitFor(() => {
      expect(setFullscreenMock).toHaveBeenCalledWith(true);
    });
  });

  it("migrates legacy localStorage profiles once and reports the result", async () => {
    const legacy = createConnections();
    localStorage.setItem("dbpinball_connections", JSON.stringify(legacy));

    const invokeMock = vi.mocked(invoke);
    let migrated = false;

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "connections_migration_status") {
        return { migratedFrom: migrated ? "localStorage@v1" : null, revision: 0, profiles: 0 };
      }
      if (command === "connections_migrate_local_storage") {
        migrated = true;
        return {
          imported: 2,
          skippedInvalid: [],
          skippedDuplicates: 0,
          skippedConflicts: [],
        };
      }
      if (command === "connections_list") {
        return legacy;
      }
      return undefined;
    });

    render(<ConnectionsPage />);

    expect(await screen.findByText(/Imported 2 connection profiles/)).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("connections_migrate_local_storage", {
      records: legacy,
    });

    // localStorage is a read-only rollback artifact: it is never deleted.
    expect(localStorage.getItem("dbpinball_connections")).not.toBeNull();
  });

  it("keeps unresolved migration items visible after a restart", async () => {
    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "connections_migration_status") {
        return {
          migratedFrom: "localStorage@v1",
          revision: 3,
          profiles: 1,
          complete: false,
          report: {
            imported: 1,
            skippedInvalid: [],
            skippedDuplicates: 0,
            skippedConflicts: [{ index: 0, id: "shared", reason: "conflict" }],
          },
        };
      }
      if (command === "connections_list") {
        return createConnections();
      }
      return undefined;
    });

    render(<ConnectionsPage />);

    expect(
      await screen.findByText(/Migration completed with unresolved items/)
    ).toBeInTheDocument();
  });

  it("shows a recoverable error instead of falling back when migration fails", async () => {
    const legacy = createConnections();
    localStorage.setItem("dbpinball_connections", JSON.stringify(legacy));

    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "connections_migration_status") {
        return { migratedFrom: null, revision: 0, profiles: 0 };
      }
      if (command === "connections_migrate_local_storage") {
        throw new Error("The connection store could not be updated.");
      }
      if (command === "connections_list") {
        return [];
      }
      return undefined;
    });

    render(<ConnectionsPage />);

    expect(await screen.findByText(/Migration did not complete/)).toBeInTheDocument();
    expect(localStorage.getItem("dbpinball_connections")).not.toBeNull();
  });
});
