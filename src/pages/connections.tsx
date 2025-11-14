import { useState, useEffect, useMemo, useImperativeHandle, forwardRef, useRef } from "react";
import type { ForwardedRef, KeyboardEvent as ReactKeyboardEvent } from "react";
import ConnectionPicker from "@/components/workspace/ConnectionPicker";
import DatabasePicker from "@/components/workspace/DatabasePicker";
import WorkspaceShell from "@/components/workspace/WorkspaceShell";
import ConnectionDialog from "@/components/workspace/ConnectionDialog";
import ConnectionsScreen from "@/components/workspace/ConnectionsScreen";
import { invoke } from "@tauri-apps/api/core";
import { useTauriWindowControls } from "@/hooks/useTauriWindowControls";
import { useWorkspaceFullscreen } from "@/hooks/useWorkspaceFullscreen";
import { useWorkspaceQuery } from "@/hooks/useWorkspaceQuery";
import type { DbConnection, WorkspaceState } from "./connections.types";
import { createInitialWorkspaceState, formatError } from "./connections.types";

export type ConnectionsPageHandle = {
  openNewConnection: (engine?: "mysql" | "postgres") => void;
};

type ConnectionsPageProps = {};

const ConnectionsPage = forwardRef(function ConnectionsPage(
  _props: ConnectionsPageProps,
  ref: ForwardedRef<ConnectionsPageHandle>,
) {
  const { handleMinimizeWindow, handleToggleMaximize, handleCloseWindow } = useTauriWindowControls();

  const [connections, setConnections] = useState<DbConnection[]>([]);
  const [open, setOpen] = useState(false);
  const [testResult, setTestResult] = useState<Record<string, boolean>>({});
  const [formData, setFormData] = useState<Partial<DbConnection>>({
    engine: "mysql",
    port: 3306,
    useSsl: false,
  });
  const [password, setPassword] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<{
    target: "row" | "list";
    id: string | null;
    open: boolean;
    position: { x: number; y: number } | null;
  }>({
    target: "list",
    id: null,
    open: false,
    position: null,
  });
  const [workspace, setWorkspace] = useState<WorkspaceState>(() => createInitialWorkspaceState());
  const [connectionPickerOpen, setConnectionPickerOpen] = useState(false);
  const [connectionPickerQuery, setConnectionPickerQuery] = useState("");
  const [connectionPickerSelectedId, setConnectionPickerSelectedId] = useState<string | null>(null);
  const [workspaceTab, setWorkspaceTab] = useState<"items" | "queries" | "history">("items");
  const [databasePickerOpen, setDatabasePickerOpen] = useState(false);
  const [databasePickerQuery, setDatabasePickerQuery] = useState("");
  const [databasePickerSelected, setDatabasePickerSelected] = useState<string | null>(null);
  const [databasePickerSuppressed, setDatabasePickerSuppressed] = useState(false);
  const [itemsSidebarQuery, setItemsSidebarQuery] = useState("");
  const [itemsFunctionsOpen, setItemsFunctionsOpen] = useState(false);
  const [itemsTablesOpen, setItemsTablesOpen] = useState(true);
  const [openTableTabs, setOpenTableTabs] = useState<string[]>([]);
  const databasePickerSearchRef = useRef<HTMLInputElement | null>(null);
  const databasePickerOpenRaf = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (databasePickerOpenRaf.current !== null) {
        cancelAnimationFrame(databasePickerOpenRaf.current);
        databasePickerOpenRaf.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (!databasePickerOpen) {
      return;
    }

    const frame = requestAnimationFrame(() => {
      databasePickerSearchRef.current?.focus();
    });

    return () => cancelAnimationFrame(frame);
  }, [databasePickerOpen]);

  useEffect(() => {
    if (workspace.databases.length === 0) {
      if (databasePickerOpenRaf.current !== null) {
        cancelAnimationFrame(databasePickerOpenRaf.current);
        databasePickerOpenRaf.current = null;
      }

      setDatabasePickerSelected(null);
      setDatabasePickerOpen(false);
      setDatabasePickerSuppressed(false);
      return;
    }

    setDatabasePickerSelected((current) => {
      if (current && workspace.databases.includes(current)) {
        return current;
      }

      if (workspace.selectedDatabase && workspace.databases.includes(workspace.selectedDatabase)) {
        return workspace.selectedDatabase;
      }

      return workspace.databases[0] ?? null;
    });

    if (workspace.open && !workspace.selectedDatabase && !databasePickerOpen && !databasePickerSuppressed) {
      setDatabasePickerQuery("");
      setDatabasePickerOpen(true);
    }
  }, [workspace.databases, workspace.selectedDatabase, workspace.open, databasePickerOpen, databasePickerSuppressed]);

  useWorkspaceFullscreen(workspace);

  const previousWorkspaceOpen = useRef(workspace.open);
  useEffect(() => {
    if (!workspace.open && previousWorkspaceOpen.current) {
      setConnectionPickerOpen(false);
    }
    previousWorkspaceOpen.current = workspace.open;
  }, [workspace.open]);

  const activeContextMenu = useMemo(() => {
    if (!contextMenu.open || contextMenu.target !== "row" || !contextMenu.id || !contextMenu.position) {
      return null;
    }

    const connection = connections.find((c) => c.id === contextMenu.id);
    if (!connection) {
      return null;
    }

    return {
      connection,
      position: contextMenu.position,
    };
  }, [connections, contextMenu]);

  useEffect(() => {
    loadConnections();
  }, []);

  const loadConnections = async () => {
    try {
      const stored = localStorage.getItem("dbpinball_connections");
      if (stored) {
        setConnections(JSON.parse(stored));
      }
    } catch (e) {
      console.error("Failed to load connections:", e);
    }
  };

  const saveConnections = (conns: DbConnection[]) => {
    localStorage.setItem("dbpinball_connections", JSON.stringify(conns));
    setConnections(conns);
  };

  const resetForm = () => {
    setFormData({ engine: "mysql", port: 3306, useSsl: false });
    setPassword("");
    setEditingId(null);
  };

  const openNewConnection = (engine: "mysql" | "postgres" = "mysql") => {
    setSelectedId(null);
    setEditingId(null);
    setFormData({ engine, port: engine === "postgres" ? 5432 : 3306, useSsl: false });
    setPassword("");
    setOpen(true);
  };

  useImperativeHandle(ref, () => ({ openNewConnection }), []);

  const openEditDialog = (conn: DbConnection) => {
    setFormData({ ...conn });
    setPassword("");
    setEditingId(conn.id);
    setOpen(true);
  };

  const handleSubmitConnection = async () => {
    if (!formData.name || !formData.host || !formData.username) {
      alert("Please fill in: Connection Name, Host, and Username");
      return;
    }

    const baseConnection: DbConnection = {
      id: editingId ?? `conn_${Date.now()}`,
      name: formData.name,
      engine: (formData.engine as "mysql" | "postgres") ?? "mysql",
      host: formData.host,
      port: formData.port || 3306,
      database: formData.database || "",
      username: formData.username,
      useSsl: formData.useSsl || false,
    };

    if (!editingId && password.trim() === "") {
      alert("Please provide a password for the new connection");
      return;
    }

    try {
      if (editingId) {
        const updatedConnections = connections.map((existing) =>
          existing.id === editingId ? baseConnection : existing
        );
        if (password.trim()) {
          await invoke("store_password", { id: editingId, password });
        }
        saveConnections(updatedConnections);
      } else {
        await invoke("store_password", { id: baseConnection.id, password });
        saveConnections([...connections, baseConnection]);
      }

      setOpen(false);
      resetForm();
    } catch (e) {
      alert(`Failed to save connection: ${e}`);
    }
  };

  const showWorkspace = (conn: DbConnection) => {
    setSelectedId(conn.id);
    setContextMenu({
      target: "list",
      id: null,
      open: false,
      position: null,
    });
    setWorkspace(() => ({
      ...createInitialWorkspaceState(),
      open: true,
      loading: true,
      connection: conn,
    }));
  };

  const handleConnect = async (conn: DbConnection) => {
    showWorkspace(conn);

    try {
      await invoke("test_conn", { conn });
      setTestResult((prev) => ({ ...prev, [conn.id]: true }));

      const databases = await invoke<string[]>("list_databases_cmd", { conn });
      setWorkspace((prev) => ({
        ...prev,
        loading: false,
        databases,
        error: databases.length === 0 ? "No databases returned." : null,
        tables: [],
        selectedDatabase: null,
        selectedTable: null,
        tablesError: null,
        queryResult: null,
        queryError: null,
        queryText: "",
        queryDirty: false,
      }));
    } catch (err) {
      const friendly = formatError(err);
      setTestResult((prev) => ({ ...prev, [conn.id]: false }));
      setWorkspace((prev) => ({
        ...prev,
        loading: false,
        error: friendly,
      }));
    } finally {
      // no-op: workspace loading state is updated above
    }
  };

  const { handleSelectDatabase, handleSelectTable, handleCloseTableTab } = useWorkspaceQuery({
    workspace,
    setWorkspace,
    setOpenTableTabs,
  });

  // Query editor handlers were removed from the Items tab; editor will live elsewhere.

  const sortedConnections = useMemo(
    () => [...connections].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" })),
    [connections],
  );

  const filteredConnections = useMemo(() => {
    const normalized = connectionPickerQuery.trim().toLowerCase();
    if (normalized.length === 0) {
      return sortedConnections;
    }

    return sortedConnections.filter((conn) => {
      const haystack = [conn.name, conn.host, conn.database, conn.username]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
      return haystack.includes(normalized);
    });
  }, [connectionPickerQuery, sortedConnections]);

  const filteredDatabases = useMemo(() => {
    const normalized = databasePickerQuery.trim().toLowerCase();
    if (normalized.length === 0) {
      return workspace.databases;
    }

    return workspace.databases.filter((dbName) => dbName.toLowerCase().includes(normalized));
  }, [databasePickerQuery, workspace.databases]);

  

  const handleShowConnectionPicker = () => {
    setConnectionPickerQuery("");
    const defaultId = workspace.connection?.id ?? sortedConnections[0]?.id ?? null;
    setConnectionPickerSelectedId(defaultId);
  };

  const handleCloseConnectionPicker = () => {
    setConnectionPickerOpen(false);
  };

  const handleConnectionPickerNavigate = (direction: 1 | -1) => {
    if (filteredConnections.length === 0) {
      return;
    }

    const currentIndex = filteredConnections.findIndex((conn) => conn.id === connectionPickerSelectedId);
    const nextIndex = (() => {
      if (currentIndex === -1) {
        return direction === 1 ? 0 : filteredConnections.length - 1;
      }

      let tentative = currentIndex + direction;
      if (tentative < 0) {
        tentative = filteredConnections.length - 1;
      } else if (tentative >= filteredConnections.length) {
        tentative = 0;
      }
      return tentative;
    })();

    setConnectionPickerSelectedId(filteredConnections[nextIndex].id);
  };

  const handleConnectionPickerSelect = (conn: DbConnection) => {
    setConnectionPickerSelectedId(conn.id);
  };

  const resolveConnectionForPicker = (candidate?: DbConnection | null) => {
    if (candidate) {
      return candidate;
    }

    const candidateId = connectionPickerSelectedId ?? filteredConnections[0]?.id ?? null;
    if (!candidateId) {
      return null;
    }

    return connections.find((item) => item.id === candidateId) ?? null;
  };

  const handleConfirmConnection = (explicit?: DbConnection | null) => {
    const target = resolveConnectionForPicker(explicit ?? null);
    if (!target) {
      return;
    }

    setConnectionPickerOpen(false);
    void handleConnect(target);
  };

  const handleConnectionPickerKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      handleConnectionPickerNavigate(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      handleConnectionPickerNavigate(-1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      handleConfirmConnection();
    } else if (event.key === "Escape") {
      event.preventDefault();
      handleCloseConnectionPicker();
    }
  };

  const handleConnectionPickerNew = () => {
    setConnectionPickerOpen(false);
    openNewConnection();
  };

  const handleShowDatabasePicker = () => {
    if (workspace.databases.length === 0) {
      return;
    }

    setDatabasePickerSuppressed(false);
    setDatabasePickerQuery("");
    const defaultSelection =
      (workspace.selectedDatabase && workspace.databases.includes(workspace.selectedDatabase)
        ? workspace.selectedDatabase
        : workspace.databases[0]) ?? null;
    setDatabasePickerSelected(defaultSelection);

    if (databasePickerOpenRaf.current !== null) {
      cancelAnimationFrame(databasePickerOpenRaf.current);
    }

    databasePickerOpenRaf.current = requestAnimationFrame(() => {
      setDatabasePickerOpen(true);
      databasePickerOpenRaf.current = null;
    });
  };

  const handleCloseDatabasePicker = () => {
    if (databasePickerOpenRaf.current !== null) {
      cancelAnimationFrame(databasePickerOpenRaf.current);
      databasePickerOpenRaf.current = null;
    }

    setDatabasePickerSuppressed(true);
    setDatabasePickerOpen(false);
    setDatabasePickerQuery("");
  };

  const handleDatabasePickerQueryChange = (value: string) => {
    setDatabasePickerQuery(value);

    const trimmed = value.trim().toLowerCase();
    const matches = trimmed
      ? workspace.databases.filter((dbName) => dbName.toLowerCase().includes(trimmed))
      : workspace.databases;

    setDatabasePickerSelected((current) => {
      if (current && matches.includes(current)) {
        return current;
      }
      return matches[0] ?? null;
    });
  };

  const handleDatabasePickerNavigate = (direction: 1 | -1) => {
    if (filteredDatabases.length === 0) {
      return;
    }

    const currentIndex = databasePickerSelected ? filteredDatabases.indexOf(databasePickerSelected) : -1;
    const nextIndex = (() => {
      if (currentIndex === -1) {
        return direction === 1 ? 0 : filteredDatabases.length - 1;
      }

      let tentative = currentIndex + direction;
      if (tentative < 0) {
        tentative = filteredDatabases.length - 1;
      } else if (tentative >= filteredDatabases.length) {
        tentative = 0;
      }
      return tentative;
    })();

    setDatabasePickerSelected(filteredDatabases[nextIndex]);
  };

  const handleDatabasePickerSelect = (dbName: string) => {
    setDatabasePickerSelected(dbName);
  };

  const handleConfirmDatabase = (dbName?: string | null) => {
    const target = dbName ?? databasePickerSelected;
    if (!target) {
      return;
    }

    setDatabasePickerSuppressed(true);
    handleCloseDatabasePicker();
    setWorkspaceTab("items");
    void handleSelectDatabase(target);
  };

  const handleDatabasePickerKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      handleDatabasePickerNavigate(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      handleDatabasePickerNavigate(-1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      handleConfirmDatabase();
    } else if (event.key === "Escape") {
      event.preventDefault();
      handleCloseDatabasePicker();
    }
  };

  useEffect(() => {
    if (!connectionPickerOpen) {
      return;
    }

    if (filteredConnections.length === 0) {
      if (connectionPickerSelectedId !== null) {
        setConnectionPickerSelectedId(null);
      }
      return;
    }

    const hasSelected = filteredConnections.some((conn) => conn.id === connectionPickerSelectedId);
    if (!hasSelected) {
      setConnectionPickerSelectedId(filteredConnections[0]?.id ?? null);
    }
  }, [connectionPickerOpen, filteredConnections, connectionPickerSelectedId]);

  const handleRootContextMenu = (event: React.MouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    setSelectedId(null);
    setContextMenu({
      target: "list",
      id: null,
      open: false,
      position: null,
    });
  };

  const closeContextMenu = () => {
    setContextMenu((prev) => ({ ...prev, open: false, id: null }));
  };

  useEffect(() => {
    if (!contextMenu.open) return;

    const handleMouseDown = (event: MouseEvent) => {
      if (event.button === 2) return;
      closeContextMenu();
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        closeContextMenu();
      }
    };

    const handleScroll = () => closeContextMenu();
    const handleResize = () => closeContextMenu();

    window.addEventListener("mousedown", handleMouseDown);
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("scroll", handleScroll, true);
    window.addEventListener("resize", handleResize);

    return () => {
      window.removeEventListener("mousedown", handleMouseDown);
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("scroll", handleScroll, true);
      window.removeEventListener("resize", handleResize);
    };
  }, [contextMenu.open]);

  const handleDeleteConnection = (id: string) => {
    if (confirm("Are you sure you want to delete this connection?")) {
      invoke("delete_password", { id }).catch(console.error);
      saveConnections(connections.filter((c) => c.id !== id));
    }
  };

  return (
    <>
      <ConnectionDialog
        open={open}
        editingId={editingId}
        formData={formData}
        password={password}
        onOpenChange={(nextOpen) => {
          setOpen(nextOpen);
          if (!nextOpen) {
            resetForm();
          }
        }}
        onChangeField={(next) => setFormData(next)}
        onChangePassword={setPassword}
        onCancel={() => {
          setOpen(false);
          resetForm();
        }}
        onSubmit={handleSubmitConnection}
      />

      <ConnectionsScreen
        connections={sortedConnections}
        selectedId={selectedId}
        testResult={testResult}
        activeContextMenu={activeContextMenu}
        onSelect={setSelectedId}
        onRootContextMenu={handleRootContextMenu}
        onRowContextMenu={(event, conn) => {
          setContextMenu({
            target: "row",
            id: conn.id,
            open: true,
            position: { x: event.clientX, y: event.clientY },
          });
        }}
        onConnect={(conn) => void handleConnect(conn)}
        onEdit={(conn) => openEditDialog(conn)}
        onDelete={(conn) => handleDeleteConnection(conn.id)}
      />

      <WorkspaceShell
        workspace={workspace}
        workspaceTab={workspaceTab}
        onWorkspaceTabChange={setWorkspaceTab}
        itemsSidebarQuery={itemsSidebarQuery}
        onItemsSidebarQueryChange={setItemsSidebarQuery}
        itemsFunctionsOpen={itemsFunctionsOpen}
        onToggleItemsFunctions={() => setItemsFunctionsOpen((prev) => !prev)}
        itemsTablesOpen={itemsTablesOpen}
        onToggleItemsTables={() => setItemsTablesOpen((prev) => !prev)}
        openTableTabs={openTableTabs}
        onSelectTable={handleSelectTable}
        onCloseTableTab={handleCloseTableTab}
        onShowConnectionPicker={handleShowConnectionPicker}
        onShowDatabasePicker={handleShowDatabasePicker}
        onMinimizeWindow={() => void handleMinimizeWindow()}
        onToggleMaximize={() => void handleToggleMaximize()}
        onCloseWindow={() => void handleCloseWindow()}
      />

      <ConnectionPicker
        open={connectionPickerOpen}
        query={connectionPickerQuery}
        onQueryChange={setConnectionPickerQuery}
        selectedId={connectionPickerSelectedId}
        onSelect={handleConnectionPickerSelect}
        onConfirm={handleConfirmConnection}
        onClose={handleCloseConnectionPicker}
        onNew={handleConnectionPickerNew}
        onKeyDown={handleConnectionPickerKeyDown}
        connections={filteredConnections}
      />

      <DatabasePicker
        open={databasePickerOpen}
        query={databasePickerQuery}
        onQueryChange={handleDatabasePickerQueryChange}
        selected={databasePickerSelected}
        onSelect={handleDatabasePickerSelect}
        onConfirm={handleConfirmDatabase}
        onClose={handleCloseDatabasePicker}
        onKeyDown={handleDatabasePickerKeyDown}
        databases={filteredDatabases}
      />
    </>
  );
});

ConnectionsPage.displayName = "ConnectionsPage";

export default ConnectionsPage;
