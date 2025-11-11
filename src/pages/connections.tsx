import { useState, useEffect, useMemo, useImperativeHandle, forwardRef } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Label } from "@/components/ui/label";
import { invoke } from "@tauri-apps/api/core";

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

export type ConnectionsPageHandle = {
  openNewConnection: (engine?: "mysql" | "postgres") => void;
};

type ConnectionsPageProps = {};

const ConnectionsPage = forwardRef<ConnectionsPageHandle, ConnectionsPageProps>(function ConnectionsPage(_props, ref) {
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
  const [workspace, setWorkspace] = useState<{
    open: boolean;
    loading: boolean;
    connection: DbConnection | null;
    databases: string[];
    error: string | null;
  }>({
    open: false,
    loading: false,
    connection: null,
    databases: [],
    error: null,
  });

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

  const closeWorkspace = () => {
    setWorkspace({ open: false, loading: false, connection: null, databases: [], error: null });
  };

  const showWorkspace = (conn: DbConnection) => {
    setSelectedId(conn.id);
    setContextMenu({
      target: "list",
      id: null,
      open: false,
      position: null,
    });
    setWorkspace({
      open: true,
      loading: true,
      connection: conn,
      databases: [],
      error: null,
    });
  };

  const handleConnect = async (conn: DbConnection) => {
    showWorkspace(conn);
    setTesting(conn.id);

    const formatError = (err: unknown) => {
      const rawMessage =
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : JSON.stringify(err);

      return /No such host is known|ENOTFOUND|EAI_AGAIN/i.test(rawMessage)
        ? `Unable to resolve host "${conn.host}". Verify the host name. Details: ${rawMessage}`
        : rawMessage;
    };

    try {
      await invoke("test_conn", { conn });
      setTestResult((prev) => ({ ...prev, [conn.id]: true }));

      const databases = await invoke<string[]>("list_databases_cmd", { conn });
      setWorkspace((prev) => ({
        ...prev,
        loading: false,
        databases,
        error: databases.length === 0 ? "No databases returned." : null,
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

  const sortedConnections = useMemo(() => {
    return [...connections].sort((a, b) => a.name.localeCompare(b.name));
  }, [connections]);

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
            <header className="connections-shell__topbar">
              <div className="connections-shell__topbar-left">
                <span className="connections-shell__logo">DB</span>
                <div className="connections-shell__breadcrumbs">
                  <span className="connections-shell__crumb">Connections</span>
                  <span className="connections-shell__crumb-separator">/</span>
                  <span className="connections-shell__crumb connections-shell__crumb--active">
                    {workspace.connection.name}
                  </span>
                </div>
              </div>
              <div className="connections-shell__topbar-right">
                {(() => {
                  const status = testResult[workspace.connection!.id];
                  const statusLabel =
                    status === undefined ? "Not tested" : status ? "Reachable" : "Failed";
                  const statusState =
                    status === undefined ? "pending" : status ? "success" : "error";

                  return (
                    <span className={`connections-shell__status-chip connections-shell__status-chip--${statusState}`}>
                      {statusLabel}
                    </span>
                  );
                })()}
                <Button variant="outline" className="connections-shell__topbar-btn" onClick={closeWorkspace}>
                  Close
                </Button>
              </div>
            </header>

            <div className="connections-shell__body">
              <section className="connections-shell__sidebar">
                <div className="connections-shell__sidebar-header">
                  <div>
                    <h2 className="connections-shell__sidebar-title">Databases</h2>
                    <p className="connections-shell__sidebar-subtitle">Fetched from server</p>
                  </div>
                </div>
                <div className="connections-shell__sidebar-body">
                  {workspace.loading ? (
                    <div className="connections-shell__sidebar-status">Loading databases…</div>
                  ) : workspace.error ? (
                    <div className="connections-shell__sidebar-error">{workspace.error}</div>
                  ) : workspace.databases.length === 0 ? (
                    <div className="connections-shell__sidebar-status">No databases returned.</div>
                  ) : (
                    <ul className="connections-shell__database-list">
                      {workspace.databases.map((dbName) => (
                        <li key={dbName} className="connections-shell__database-item">
                          <span>{dbName}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              </section>

              <main className="connections-shell__workspace">
                <div className="connections-shell__workspace-card">
                  <header className="connections-shell__workspace-card-header">
                    <h3>{workspace.connection.name}</h3>
                    <span className="connections-shell__workspace-connection">
                      {workspace.connection.username}@{workspace.connection.host}:{workspace.connection.port}
                    </span>
                  </header>
                  <div className="connections-shell__workspace-card-body">
                    {workspace.loading ? (
                      <div className="connections-shell__workspace-status">Testing connection…</div>
                    ) : workspace.error ? (
                      <div className="connections-shell__workspace-error">{workspace.error}</div>
                    ) : (
                      <div className="connections-shell__workspace-placeholder">
                        Select a database from the left to explore its tables.
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
    </>
  );
});

ConnectionsPage.displayName = "ConnectionsPage";

export default ConnectionsPage;
