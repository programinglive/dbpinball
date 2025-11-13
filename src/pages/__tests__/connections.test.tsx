import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
    render(<ConnectionsPage />);

    expect(await screen.findByText("No connections yet.")).toBeInTheDocument();
    expect(
      await screen.findByText("Create a connection to get started.")
    ).toBeInTheDocument();
  });

  it("opens workspace overlay and lists databases after connecting", async () => {
    const connections = createConnections();
    localStorage.setItem("dbpinball_connections", JSON.stringify(connections));

    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "test_conn") {
        return;
      }
      if (command === "list_databases_cmd") {
        return ["main", "reporting"];
      }
      return;
    });

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

    const [openListButton] = screen.getAllByRole("button", { name: "Open connection list" });
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
    localStorage.setItem("dbpinball_connections", JSON.stringify(connections));

    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string, payload: unknown) => {
      if (command === "test_conn") {
        expect(payload).toMatchObject({ conn: connections[0] });
        return;
      }
      if (command === "list_databases_cmd") {
        return ["main", "reporting"];
      }
      return;
    });

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
    localStorage.setItem("dbpinball_connections", JSON.stringify(connections));

    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async () => undefined);

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
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("deletes connection via context menu when confirmed", async () => {
    const connections = createConnections();
    localStorage.setItem("dbpinball_connections", JSON.stringify(connections));

    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "delete_password") {
        return;
      }
      return;
    });

    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<ConnectionsPage />);

    const analyticsCard = await screen.findByText("Analytics");
    fireEvent.contextMenu(analyticsCard, { clientX: 80, clientY: 80 });

    const deleteButton = await screen.findByRole("button", { name: "Delete" });

    await act(async () => {
      fireEvent.mouseDown(deleteButton);
    });

    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("delete_password", { id: "conn_1" });

    await waitFor(() => {
      expect(screen.queryByText("Analytics")).not.toBeInTheDocument();
    });

    confirmSpy.mockRestore();
  });

  it("shows friendly error messaging when connection test fails", async () => {
    const connections = createConnections();
    localStorage.setItem("dbpinball_connections", JSON.stringify(connections));

    const invokeMock = vi.mocked(invoke);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "test_conn") {
        throw new Error("No such host is known");
      }
      return;
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
});
