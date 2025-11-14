import { useState, useEffect, useMemo, useImperativeHandle, forwardRef, useCallback, useRef } from "react";
import type { ForwardedRef, KeyboardEvent as ReactKeyboardEvent } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogOverlay, DialogPortal, DialogTitle } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { invoke } from "@tauri-apps/api/core";
import { Settings, Minus, Square, X, PlugZap, Database, ChevronRight, Search } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface DbConnection {
  id: string;
  name: string;
  engine: "mysql" | "postgres";
  host: string;
  port: number;
  database: string;
  username: string;
  useSsl: boolean;
}

type QueryResultRow = (string | null)[];

type QueryResultPayload = {
  columns: string[];
  rows: QueryResultRow[];
};

type WorkspaceState = {
  open: boolean;
  loading: boolean;
  connection: DbConnection | null;
  databases: string[];
  tables: string[];
  selectedDatabase: string | null;
  selectedTable: string | null;
  tablesLoading: boolean;
  tablesError: string | null;
  queryText: string;
  queryLimit: number;
  queryLimitInput: string;
  queryRunning: boolean;
  queryDirty: boolean;
  queryResult: QueryResultPayload | null;
  queryError: string | null;
  error: string | null;
};

const formatError = (err: unknown) => {
  const rawMessage =
    typeof err === "string"
      ? err
      : err instanceof Error
        ? err.message
        : JSON.stringify(err);

  return /No such host is known|ENOTFOUND|EAI_AGAIN/i.test(rawMessage)
    ? rawMessage.replace(/No such host is known|ENOTFOUND|EAI_AGAIN/i, (match) =>
        `Unable to resolve host. Details: ${match}`
      )
    : rawMessage;
};

const createInitialWorkspaceState = (): WorkspaceState => ({
  open: false,
  loading: false,
  connection: null,
  databases: [],
  tables: [],
  selectedDatabase: null,
  selectedTable: null,
  tablesLoading: false,
  tablesError: null,
  queryText: "",
  queryLimit: 100,
  queryLimitInput: "100",
  queryRunning: false,
  queryDirty: false,
  queryResult: null,
  queryError: null,
  error: null,
});

export type ConnectionsPageHandle = {
  openNewConnection: (engine?: "mysql" | "postgres") => void;
};

type ConnectionsPageProps = {};

const ConnectionsPage = forwardRef(function ConnectionsPage(
  _props: ConnectionsPageProps,
  ref: ForwardedRef<ConnectionsPageHandle>,
) {
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

  const [connections, setConnections] = useState<DbConnection[]>([]);
  const [open, setOpen] = useState(false);
  const [testing, setTesting] = useState<string | null>(null);
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
  const connectionPickerSearchRef = useRef<HTMLInputElement | null>(null);
  const databasePickerSearchRef = useRef<HTMLInputElement | null>(null);
  const connectionPickerOpenRaf = useRef<number | null>(null);
  const databasePickerOpenRaf = useRef<number | null>(null);

  useEffect(() => {
    return;
  }, [connectionPickerOpen]);

  useEffect(() => {
    return () => {
      if (connectionPickerOpenRaf.current !== null) {
        cancelAnimationFrame(connectionPickerOpenRaf.current);
        connectionPickerOpenRaf.current = null;
      }
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
    setTesting(conn.id);

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
      setTesting(null);
    }
  };

  const quoteIdentifier = (engine: DbConnection["engine"], value: string) => {
    const trimmed = value.trim();
    if (engine === "postgres") {
      return `"${trimmed.replace(/"/g, '""')}"`;
    }
    return `\`${trimmed.replace(/`/g, "``")}\``;
  };

  const buildSelectQuery = (engine: DbConnection["engine"], table: string, limit: number) => {
    const limitClause = Number.isFinite(limit) && limit > 0 ? ` LIMIT ${limit}` : "";
    return `SELECT * FROM ${quoteIdentifier(engine, table)}${limitClause};`;
  };

  const handleSelectDatabase = async (dbName: string) => {
    if (!workspace.connection) {
      return;
    }

    const conn = workspace.connection;
    const connectionId = conn.id;

    setWorkspace((prev) => ({
      ...prev,
      selectedDatabase: dbName,
      selectedTable: null,
      tables: [],
      tablesLoading: true,
      tablesError: null,
      queryResult: null,
      queryError: null,
    }));

    try {
      const tables = await invoke<string[]>("list_tables_cmd", { conn, database: dbName });
      setWorkspace((prev) => {
        if (!prev.connection || prev.connection.id !== connectionId || prev.selectedDatabase !== dbName) {
          return prev;
        }

        const hasTables = tables.length > 0;
        const firstTable = hasTables ? tables[0] : null;
        const shouldResetQuery = !prev.queryDirty;
        const nextQueryText = firstTable && shouldResetQuery
          ? buildSelectQuery(prev.connection.engine, firstTable, prev.queryLimit)
          : prev.queryText;

        return {
          ...prev,
          tables,
          tablesLoading: false,
          tablesError: hasTables ? null : "No tables found in this database.",
          selectedTable: firstTable,
          queryText: nextQueryText,
          queryDirty: shouldResetQuery ? false : prev.queryDirty,
          queryResult: null,
        };
      });
    } catch (err) {
      setWorkspace((prev) => {
        if (!prev.connection || prev.connection.id !== connectionId || prev.selectedDatabase !== dbName) {
          return prev;
        }

        return {
          ...prev,
          tablesLoading: false,
          tablesError: formatError(err),
        };
      });
    }
  };

  const handleSelectTable = (tableName: string) => {
    setWorkspace((prev) => {
      if (!prev.connection) {
        return prev;
      }

      const shouldResetQuery = !prev.queryDirty;
      const nextQueryText = shouldResetQuery
        ? buildSelectQuery(prev.connection.engine, tableName, prev.queryLimit)
        : prev.queryText;

      return {
        ...prev,
        selectedTable: tableName,
        queryText: nextQueryText,
        queryDirty: shouldResetQuery ? false : prev.queryDirty,
        // we will immediately run a fresh preview below when resetting the query
        queryResult: shouldResetQuery ? null : prev.queryResult,
        queryError: null,
      };
    });

    // Auto-run a preview for the selected table, similar to TablePlus behaviour.
    setWorkspace((prev) => {
      if (!prev.connection || !prev.selectedDatabase) {
        return prev;
      }

      const sql = buildSelectQuery(prev.connection.engine, tableName, prev.queryLimit);

      // Kick off the async query; result handling mirrors handleRunQuery.
      (async () => {
        const conn = prev.connection!;
        const connectionId = conn.id;
        const database = prev.selectedDatabase!;

        try {
          const result = await invoke<QueryResultPayload>("run_query_cmd", {
            conn,
            database,
            sql,
            limit: prev.queryLimit,
          });

          setWorkspace((current) => {
            if (!current.connection || current.connection.id !== connectionId || current.selectedDatabase !== database) {
              return current;
            }

            return {
              ...current,
              queryRunning: false,
              queryResult: result,
              queryError: null,
            };
          });
        } catch (err) {
          setWorkspace((current) => ({
            ...current,
            queryRunning: false,
            queryError: formatError(err),
          }));
        }
      })().catch(console.error);

      return {
        ...prev,
        queryRunning: true,
        queryError: null,
      };
    });
  };

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

  const filteredTables = useMemo(() => {
    const normalized = itemsSidebarQuery.trim().toLowerCase();
    if (normalized.length === 0) {
      return workspace.tables;
    }

    return workspace.tables.filter((tableName) => tableName.toLowerCase().includes(normalized));
  }, [itemsSidebarQuery, workspace.tables]);

  const handleShowConnectionPicker = () => {
    setConnectionPickerQuery("");
    const defaultId = workspace.connection?.id ?? sortedConnections[0]?.id ?? null;
    setConnectionPickerSelectedId(defaultId);
  };

  const handleCloseConnectionPicker = () => {
    if (connectionPickerOpenRaf.current !== null) {
      cancelAnimationFrame(connectionPickerOpenRaf.current);
      connectionPickerOpenRaf.current = null;
    }
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

  useEffect(() => {
    if (!connectionPickerOpen) {
      return;
    }

    const frame = requestAnimationFrame(() => {
      connectionPickerSearchRef.current?.focus();
    });

    return () => cancelAnimationFrame(frame);
  }, [connectionPickerOpen]);

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
      <div className="connections-screen">

        <Dialog
          open={open}
          onOpenChange={(nextOpen) => {
            setOpen(nextOpen);
            if (!nextOpen) {
              resetForm();
            }
          }}
        >
          <DialogContent className="connection-dialog">
            <DialogHeader className="connection-dialog__header">
              <DialogTitle className="connection-dialog__title">
                {editingId ? "Edit Database Connection" : "Add Database Connection"}
              </DialogTitle>
              <DialogDescription className="connection-dialog__description">
                {editingId ? "Update the connection details." : "Create a new database connection."}
              </DialogDescription>
            </DialogHeader>

            <div className="connection-form">
              <section className="connection-form__section">
                <header className="connection-form__section-header">
                  <h3 className="connection-form__section-title">Connection</h3>
                  <p className="connection-form__section-description">
                    Describe the database and where it can be reached.
                  </p>
                </header>

                <div className="connection-form__row">
                  <div className="connection-field connection-field--grow">
                    <Label htmlFor="name">Connection Name</Label>
                    <Input
                      id="name"
                      placeholder="My Database"
                      value={formData.name || ""}
                      className="connection-input"
                      onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                    />
                  </div>
                  <div className="connection-field connection-field--slim">
                    <Label htmlFor="engine">Database Engine</Label>
                    <Select value={formData.engine} onValueChange={(v) => setFormData({ ...formData, engine: v as "mysql" | "postgres" })}>
                      <SelectTrigger className="connection-select">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="mysql">MySQL</SelectItem>
                        <SelectItem value="postgres">PostgreSQL</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>

                <div className="connection-form__row">
                  <div className="connection-field connection-field--grow">
                    <Label htmlFor="host">Host</Label>
                    <Input
                      id="host"
                      placeholder="localhost"
                      value={formData.host || ""}
                      className="connection-input"
                      onChange={(e) => setFormData({ ...formData, host: e.target.value })}
                    />
                  </div>
                  <div className="connection-field connection-field--compact">
                    <Label htmlFor="port">Port</Label>
                    <Input
                      id="port"
                      type="number"
                      value={formData.port || ""}
                      className="connection-input"
                      onChange={(e) => setFormData({ ...formData, port: parseInt(e.target.value) })}
                    />
                  </div>
                </div>

                <div className="connection-form__row connection-form__row--single">
                  <div className="connection-field">
                    <Label htmlFor="database">Database</Label>
                    <Input
                      id="database"
                      placeholder="mydb"
                      value={formData.database || ""}
                      className="connection-input"
                      onChange={(e) => setFormData({ ...formData, database: e.target.value })}
                    />
                  </div>
                </div>
              </section>

              <section className="connection-form__section">
                <header className="connection-form__section-header">
                  <h3 className="connection-form__section-title">Authentication</h3>
                  <p className="connection-form__section-description">
                    Provide credentials DB Pinball should use.
                  </p>
                </header>

                <div className="connection-form__row">
                  <div className="connection-field connection-field--grow">
                    <Label htmlFor="username">Username</Label>
                    <Input
                      id="username"
                      placeholder="root"
                      value={formData.username || ""}
                      className="connection-input"
                      onChange={(e) => setFormData({ ...formData, username: e.target.value })}
                    />
                  </div>
                  <div className="connection-field connection-field--grow">
                    <Label htmlFor="password">Password {editingId ? "(leave blank to keep existing)" : ""}</Label>
                    <Input
                      id="password"
                      type="password"
                      placeholder="••••••••"
                      value={password}
                      className="connection-input"
                      onChange={(e) => setPassword(e.target.value)}
                    />
                  </div>
                </div>
              </section>

              <footer className="connection-form__footer">
                <span className="connection-form__footer-hint">Save to persist changes or test the connection from the list.</span>
                <div className="connection-form__actions">
                  <Button onClick={handleSubmitConnection} className="connection-btn connection-btn--primary">
                    {editingId ? "Update Connection" : "Save Connection"}
                  </Button>
                  <Button
                    variant="outline"
                    onClick={() => {
                      setOpen(false);
                      resetForm();
                    }}
                    className="connection-btn connection-btn--ghost"
                  >
                    Cancel
                  </Button>
                </div>
              </footer>
            </div>
          </DialogContent>
        </Dialog>

        <div className="connections-list" onContextMenu={handleRootContextMenu}>
          {sortedConnections.length === 0 ? (
            <div className="connections-list__empty">
              <p>No connections yet.</p>
              <p className="connections-list__empty-sub">Create a connection to get started.</p>
            </div>
          ) : (
            sortedConnections.map((conn) => {
              const status = testResult[conn.id];
              const statusLabel =
                status === undefined ? "Not tested" : status ? "Reachable" : "Failed";
              const statusState =
                status === undefined ? "pending" : status ? "success" : "error";
              const engineBadge = conn.engine === "postgres" ? "Pg" : "My";

              return (
                <Card
                  key={conn.id}
                  className={`connection-row${selectedId === conn.id ? " connection-row--selected" : ""}`}
                  onClick={() => setSelectedId(conn.id)}
                  onDoubleClick={() => handleConnect(conn)}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    setSelectedId(conn.id);
                    setContextMenu({
                      target: "row",
                      id: conn.id,
                      open: true,
                      position: { x: event.clientX, y: event.clientY },
                    });
                  }}
                >
                  <CardHeader className="connection-row__header">
                    <div className="connection-row__content">
                      <div className="connection-row__avatar">{engineBadge}</div>
                      <div className="connection-row__text">
                        <div className="connection-row__title-line">
                          <CardTitle className="connection-row__title">{conn.name}</CardTitle>
                          <span className={`connection-row__status-text connection-row__status-text--${statusState}`}>
                            {statusLabel}
                          </span>
                        </div>
                        <CardDescription className="connection-row__description">
                          {conn.username}@{conn.host}:{conn.port}
                          {conn.database ? ` · ${conn.database}` : ""}
                          {conn.useSsl && <span className="connection-row__ssl"> · SSL</span>}
                        </CardDescription>
                      </div>
                    </div>
                  </CardHeader>
                </Card>
              );
            })
          )}
        </div>

        {activeContextMenu && (
          <div
            className="connection-menu"
            style={{
              position: "fixed",
              left: activeContextMenu.position.x,
              top: activeContextMenu.position.y,
            }}
          >
            <button
              type="button"
              className="connection-menu__item"
              onMouseDown={(event) => {
                event.preventDefault();
                closeContextMenu();
                void handleConnect(activeContextMenu.connection);
              }}
              disabled={testing === activeContextMenu.connection.id}
            >
              {testing === activeContextMenu.connection.id ? "Connecting…" : "Connect"}
            </button>
            <button
              type="button"
              className="connection-menu__item"
              onMouseDown={(event) => {
                event.preventDefault();
                closeContextMenu();
                openEditDialog(activeContextMenu.connection);
              }}
            >
              Edit…
            </button>
            <button
              type="button"
              className="connection-menu__item connection-menu__item--danger"
              onMouseDown={(event) => {
                event.preventDefault();
                closeContextMenu();
                handleDeleteConnection(activeContextMenu.connection.id);
              }}
            >
              Delete
            </button>
          </div>
        )}
      </div>

      {workspace.open && workspace.connection && (
        <div className="workspace-overlay" role="dialog" aria-modal="true">
          <div className="connections-shell">
            <header className="connections-shell__topbar" data-tauri-drag-region>
              <div className="connections-shell__topbar-section connections-shell__topbar-section--brand">
                <button
                  type="button"
                  className="connections-shell__icon-btn connections-shell__icon-btn--brand"
                  aria-label="Open connection list"
                  onClick={handleShowConnectionPicker}
                  data-tauri-drag-region="false"
                >
                  <PlugZap size={16} />
                </button>
                <button
                  type="button"
                  className="connections-shell__icon-btn"
                  aria-label="Open database picker"
                  onClick={handleShowDatabasePicker}
                  disabled={workspace.databases.length === 0}
                  data-tauri-drag-region="false"
                >
                  <Database size={14} />
                </button>
              </div>

              <div className="connections-shell__topbar-section connections-shell__topbar-section--info">
                <div className="connections-shell__session-line" aria-label="Current connection summary">
                  <span className="connections-shell__session-env">LOCAL</span>
                  <span className="connections-shell__session-separator">|</span>
                  <span className="connections-shell__session-meta">
                    {workspace.connection.engine === "postgres" ? "PostgreSQL" : "MySQL"}
                  </span>
                  <span className="connections-shell__session-separator">·</span>
                  <span className="connections-shell__session-path">
                    <span className="connections-shell__session-path--muted">{workspace.connection.username}</span>
                    <span className="connections-shell__session-path-separator">@</span>
                    <span>{workspace.connection.host}:{workspace.connection.port}</span>
                    {workspace.selectedDatabase && (
                      <>
                        <span className="connections-shell__session-path-separator">·</span>
                        <span>{workspace.selectedDatabase}</span>
                      </>
                    )}
                  </span>
                </div>
              </div>

              <div className="connections-shell__topbar-section connections-shell__topbar-section--actions">
                <button type="button" className="connections-shell__icon-btn" aria-label="Settings" data-tauri-drag-region="false">
                  <Settings size={16} />
                </button>

                <button
                  type="button"
                  className="connections-shell__icon-btn"
                  aria-label="Minimize"
                  onClick={() => void handleMinimizeWindow()}
                  data-tauri-drag-region="false"
                >
                  <Minus size={14} />
                </button>

                <button
                  type="button"
                  className="connections-shell__icon-btn"
                  aria-label="Toggle maximize"
                  onClick={() => void handleToggleMaximize()}
                  data-tauri-drag-region="false"
                >
                  <Square size={12} />
                </button>

                <button
                  type="button"
                  className="connections-shell__icon-btn"
                  aria-label="Close"
                  onClick={() => void handleCloseWindow()}
                  data-tauri-drag-region="false"
                >
                  <X size={12} />
                </button>
              </div>
            </header>

            <div className="connections-shell__body">
              <section className="connections-shell__sidebar">
                <div className="connections-shell__sidebar-header">
                  <div className="workspace-panel__tabs">
                    <button
                      type="button"
                      className={`workspace-panel__tab${workspaceTab === "items" ? " workspace-panel__tab--active" : ""}`}
                      onClick={() => setWorkspaceTab("items")}
                    >
                      Items
                    </button>
                    <button
                      type="button"
                      className={`workspace-panel__tab${workspaceTab === "queries" ? " workspace-panel__tab--active" : ""}`}
                      onClick={() => setWorkspaceTab("queries")}
                    >
                      Queries
                    </button>
                    <button
                      type="button"
                      className={`workspace-panel__tab${workspaceTab === "history" ? " workspace-panel__tab--active" : ""}`}
                      onClick={() => setWorkspaceTab("history")}
                    >
                      History
                    </button>
                  </div>
                </div>
                <div className="connections-shell__sidebar-body">
                  {workspaceTab === "items" && (
                    workspace.databases.length === 0 ? (
                      <div className="connections-shell__sidebar-status connections-shell__sidebar-status--empty">
                        <Database size={36} strokeWidth={1.2} aria-hidden="true" />
                        <p>No databases available.</p>
                        <span>Create or connect to a database to browse its items.</span>
                      </div>
                    ) : workspace.selectedDatabase ? (
                      <div className="items-sidebar">
                        <div className="items-sidebar__search">
                          <Search size={14} aria-hidden="true" />
                          <Input
                            value={itemsSidebarQuery}
                            onChange={(event) => setItemsSidebarQuery(event.target.value)}
                            placeholder="Search items"
                            className="items-sidebar__search-input"
                          />
                        </div>

                        <ScrollArea className="items-sidebar__scroll">
                          <div className="items-sidebar__section">
                            <button
                              type="button"
                              className="items-sidebar__section-header"
                              onClick={() => setItemsFunctionsOpen((prev) => !prev)}
                            >
                              <span>Functions</span>
                              <ChevronRight
                                size={14}
                                className={`items-sidebar__section-icon${itemsFunctionsOpen ? " items-sidebar__section-icon--open" : ""}`}
                              />
                            </button>

                            {itemsFunctionsOpen && (
                              <div className="items-sidebar__section-body">
                                <div className="items-sidebar__empty">
                                  <Database size={32} strokeWidth={1.3} aria-hidden="true" />
                                  <p>No functions yet.</p>
                                  <span>Create a function to see it here.</span>
                                </div>
                              </div>
                            )}
                          </div>

                          <div className="items-sidebar__section">
                            <button
                              type="button"
                              className="items-sidebar__section-header"
                              onClick={() => setItemsTablesOpen((prev) => !prev)}
                            >
                              <span>Tables</span>
                              <ChevronRight
                                size={14}
                                className={`items-sidebar__section-icon${itemsTablesOpen ? " items-sidebar__section-icon--open" : ""}`}
                              />
                            </button>

                            {itemsTablesOpen && (
                              <div className="items-sidebar__section-body">
                                {workspace.tablesError ? (
                                  <div className="items-sidebar__status items-sidebar__status--error">{workspace.tablesError}</div>
                                ) : workspace.tablesLoading ? (
                                  <div className="items-sidebar__status">Loading tables…</div>
                                ) : filteredTables.length === 0 ? (
                                  <div className="items-sidebar__empty">
                                    <Database size={32} strokeWidth={1.3} aria-hidden="true" />
                                    <p>No tables match your search.</p>
                                    <span>Try a different search or refresh.</span>
                                  </div>
                                ) : (
                                  <ul className="items-sidebar__list">
                                    {filteredTables.map((tableName) => {
                                      const isSelected = workspace.selectedTable === tableName;
                                      return (
                                        <li key={tableName}>
                                          <button
                                            type="button"
                                            className={`items-sidebar__item${isSelected ? " items-sidebar__item--active" : ""}`}
                                            onClick={() => handleSelectTable(tableName)}
                                          >
                                            <span>{tableName}</span>
                                          </button>
                                        </li>
                                      );
                                    })}
                                  </ul>
                                )}
                              </div>
                            )}
                          </div>
                        </ScrollArea>
                      </div>
                    ) : (
                      <div className="connections-shell__sidebar-status connections-shell__sidebar-status--empty">
                        <Database size={36} strokeWidth={1.2} aria-hidden="true" />
                        <p>Items content coming soon.</p>
                        <span>Select a database from the left to explore its tables.</span>
                      </div>
                    )
                  )}
                  {workspaceTab === "queries" && (
                    <div className="connections-shell__sidebar-status connections-shell__sidebar-status--empty">
                      <Database size={36} strokeWidth={1.2} aria-hidden="true" />
                      <p>No saved queries yet.</p>
                      <span>Run a query and save it to see it here.</span>
                    </div>
                  )}
                  {workspaceTab === "history" && (
                    <div className="connections-shell__sidebar-status connections-shell__sidebar-status--empty">
                      <Database size={36} strokeWidth={1.2} aria-hidden="true" />
                      <p>No query history yet.</p>
                      <span>Your recent queries will appear here.</span>
                    </div>
                  )}
                </div>
              </section>

              <main className="connections-shell__workspace">
                <div className="connections-shell__workspace-card">
                  <div className="connections-shell__workspace-card-body">
                    {workspace.loading ? (
                      <div className="connections-shell__workspace-status">Testing connection…</div>
                    ) : workspace.error ? (
                      <div className="connections-shell__workspace-error">{workspace.error}</div>
                    ) : workspace.selectedDatabase ? (
                      <div className="workspace-panel">
                        <div className="workspace-panel__summary">
                          <div>
                            <span className="workspace-panel__summary-label">Database</span>
                            <span className="workspace-panel__summary-value">{workspace.selectedDatabase}</span>
                          </div>
                          <div>
                            <span className="workspace-panel__summary-label">Table</span>
                            <span className="workspace-panel__summary-value">
                              {workspace.selectedTable ?? "Select a table"}
                            </span>
                          </div>
                        </div>

                        {workspaceTab === "items" && (
                        <div className="workspace-panel__content workspace-panel__content--single">
                          <section className="workspace-panel__query">
                            {workspace.queryError && (
                              <p className="workspace-panel__error" role="alert">
                                {workspace.queryError}
                              </p>
                            )}

                            <div className="workspace-panel__result">
                              {workspace.queryRunning && !workspace.queryResult ? (
                                <div className="workspace-panel__status">Running query</div>
                              ) : null}

                              {!workspace.queryRunning && workspace.queryResult && workspace.queryResult.columns.length > 0 && (
                                <ScrollArea className="workspace-panel__result-scroll">
                                  <Table className="workspace-panel__result-table">
                                    <TableHeader>
                                      <TableRow>
                                        {workspace.queryResult.columns.map((column, columnIndex) => (
                                          <TableHead key={`${column}-${columnIndex}`}>{column}</TableHead>
                                        ))}
                                      </TableRow>
                                    </TableHeader>
                                    <TableBody>
                                      {workspace.queryResult.rows.map((row, rowIndex) => (
                                        <TableRow key={rowIndex}>
                                          {row.map((value, cellIndex) => (
                                            <TableCell key={cellIndex}>
                                              {value === null ? (
                                                <span className="workspace-panel__result-null">NULL</span>
                                              ) : (
                                                value
                                              )}
                                            </TableCell>
                                          ))}
                                        </TableRow>
                                      ))}
                                    </TableBody>
                                  </Table>
                                </ScrollArea>
                              )}
                            </div>
                          </section>
                        </div>
                        )}

                        {workspaceTab === "queries" && (
                        <div className="workspace-panel__empty-state" />
                        )}

                        {workspaceTab === "history" && (
                        <div className="workspace-panel__empty-state" />
                        )}
                      </div>
                    ) : (
                      <div className="connections-shell__workspace-placeholder">
                        Choose a database to run queries and explore its data.
                      </div>
                    )}
                  </div>
                </div>
              </main>

              <aside className="connections-shell__inspector">
                <div className="connections-shell__inspector-header">Details</div>
                <div className="connections-shell__inspector-body">
                  <ul className="connections-shell__inspector-list">
                    <li>
                      <span className="connections-shell__inspector-label">Host</span>
                      <span className="connections-shell__inspector-value">{workspace.connection.host}</span>
                    </li>
                    <li>
                      <span className="connections-shell__inspector-label">Port</span>
                      <span className="connections-shell__inspector-value">{workspace.connection.port}</span>
                    </li>
                    <li>
                      <span className="connections-shell__inspector-label">User</span>
                      <span className="connections-shell__inspector-value">{workspace.connection.username}</span>
                    </li>
                    <li>
                      <span className="connections-shell__inspector-label">Database</span>
                      <span className="connections-shell__inspector-value">
                        {workspace.connection.database || "Not specified"}
                      </span>
                    </li>
                    <li>
                      <span className="connections-shell__inspector-label">SSL</span>
                      <span className="connections-shell__inspector-value">
                        {workspace.connection.useSsl ? "Enabled" : "Disabled"}
                      </span>
                    </li>
                  </ul>
                </div>
              </aside>

            </div>
          </div>
        </div>
      )}

      <Dialog open={connectionPickerOpen}>
        <DialogPortal>
          <DialogOverlay
            className="connection-picker__overlay"
            style={{ zIndex: 4000 }}
          />
          <DialogContent
            className="connection-picker"
            style={{ zIndex: 4100, width: "460px", maxWidth: "460px" }}
            hideClose
            onEscapeKeyDown={(event) => {
              event.preventDefault();
              handleCloseConnectionPicker();
            }}
            onPointerDownOutside={(event) => {
              event.preventDefault();
              handleCloseConnectionPicker();
            }}
          >
            <div className="connection-picker__titlebar">
              <DialogTitle className="connection-picker__title-text">Open connection</DialogTitle>
              <div className="connection-picker__titlebar-actions">
                <button
                  type="button"
                  className="connection-picker__titlebar-btn connection-picker__titlebar-btn--close"
                  aria-label="Close connection picker"
                  onClick={handleCloseConnectionPicker}
                  data-tauri-drag-region="false"
                >
                  <X size={12} />
                </button>
              </div>
            </div>
            <DialogDescription className="sr-only">
              Search and pick from saved database connections. Use arrow keys to move and Enter to connect.
            </DialogDescription>

            <div className="connection-picker__body">
              <Input
                ref={connectionPickerSearchRef}
                placeholder="Search for connection… (Ctrl F)"
                value={connectionPickerQuery}
                onChange={(event) => setConnectionPickerQuery(event.target.value)}
                onKeyDown={handleConnectionPickerKeyDown}
                className="connection-picker__search"
                aria-label="Search connections"
              />

              <ScrollArea className="connection-picker__list" role="listbox" aria-label="Connections">
                {filteredConnections.length === 0 ? (
                  <div className="connection-picker__empty">No matching connections.</div>
                ) : (
                  <ul>
                    {filteredConnections.map((conn) => {
                      const isSelected = connectionPickerSelectedId === conn.id;
                      const badge = conn.engine === "postgres" ? "Pg" : "My";
                      const description = conn.database ? `${conn.host} · ${conn.database}` : conn.host;
                      const statusChip = conn.engine === "postgres" ? "(PostgreSQL)" : "(MySQL)";

                      return (
                        <li key={conn.id}>
                          <button
                            type="button"
                            role="option"
                            aria-selected={isSelected}
                            className={`connection-picker__item${isSelected ? " connection-picker__item--active" : ""}`}
                            onMouseEnter={() => handleConnectionPickerSelect(conn)}
                            onFocus={() => handleConnectionPickerSelect(conn)}
                            onClick={() => handleConfirmConnection(conn)}
                          >
                            <span className="connection-picker__item-badge">{badge}</span>
                            <span className="connection-picker__item-content">
                              <span className="connection-picker__item-title">
                                {conn.name} <span className="connection-picker__item-tag">{statusChip}</span>
                              </span>
                              <span className="connection-picker__item-description">{description}</span>
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </ScrollArea>
            </div>

            <div className="connection-picker__footer">
              <Button variant="ghost" onClick={handleConnectionPickerNew} className="connection-picker__footer-btn">
                New…
              </Button>
              <Button
                onClick={() => handleConfirmConnection()}
                disabled={filteredConnections.length === 0}
                className="connection-picker__footer-btn connection-picker__footer-btn--primary"
              >
                Open
              </Button>
            </div>
          </DialogContent>
        </DialogPortal>
      </Dialog>

      <Dialog open={databasePickerOpen}>
        <DialogPortal>
          <DialogOverlay className="database-picker__overlay" style={{ zIndex: 5000 }} />
          <DialogContent
            className="database-picker"
            style={{ zIndex: 5010, width: "420px", maxWidth: "420px" }}
            hideClose
            onEscapeKeyDown={(event) => {
              event.preventDefault();
              handleCloseDatabasePicker();
            }}
            onPointerDownOutside={(event) => {
              event.preventDefault();
              handleCloseDatabasePicker();
            }}
          >
            <div className="database-picker__titlebar">
              <DialogTitle className="database-picker__title-text">Open database</DialogTitle>
              <div className="database-picker__titlebar-actions">
                <button
                  type="button"
                  className="database-picker__titlebar-btn database-picker__titlebar-btn--close"
                  aria-label="Close database picker"
                  onClick={handleCloseDatabasePicker}
                >
                  <X size={12} />
                </button>
              </div>
            </div>
            <DialogDescription className="sr-only">
              Search and select a database to load its tables.
            </DialogDescription>

            <div className="database-picker__body">
              <Input
                ref={databasePickerSearchRef}
                placeholder="Search for database…"
                value={databasePickerQuery}
                onChange={(event) => handleDatabasePickerQueryChange(event.target.value)}
                onKeyDown={handleDatabasePickerKeyDown}
                className="database-picker__search"
                aria-label="Search databases"
              />

              <ScrollArea className="database-picker__list" role="listbox" aria-label="Databases">
                {filteredDatabases.length === 0 ? (
                  <div className="database-picker__empty">No databases found.</div>
                ) : (
                  <ul>
                    {filteredDatabases.map((dbName) => {
                      const isActive = databasePickerSelected === dbName;

                      return (
                        <li key={dbName}>
                          <button
                            type="button"
                            role="option"
                            aria-selected={isActive}
                            className={`database-picker__item${isActive ? " database-picker__item--active" : ""}`}
                            onMouseEnter={() => handleDatabasePickerSelect(dbName)}
                            onFocus={() => handleDatabasePickerSelect(dbName)}
                            onClick={() => handleConfirmDatabase(dbName)}
                          >
                            <span className="database-picker__item-label">{dbName}</span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </ScrollArea>
            </div>

            <div className="database-picker__footer">
              <Button
                onClick={() => handleConfirmDatabase()}
                disabled={filteredDatabases.length === 0}
                className="database-picker__footer-btn database-picker__footer-btn--primary"
              >
                Open
              </Button>
            </div>
          </DialogContent>
        </DialogPortal>
      </Dialog>
    </>
  );
});

ConnectionsPage.displayName = "ConnectionsPage";

export default ConnectionsPage;
