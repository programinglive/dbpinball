import { Settings, Minus, Square, X, PlugZap, Database } from "lucide-react";
import { useEffect } from "react";
import type { WorkspaceState } from "@/pages/connections.types";
import ItemsSidebar from "@/components/workspace/ItemsSidebar";
import TableTabs from "@/components/workspace/TableTabs";
import ResultGrid from "@/components/workspace/ResultGrid";

export type WorkspaceTab = "items" | "queries" | "history";

interface WorkspaceShellProps {
  workspace: WorkspaceState;
  workspaceTab: WorkspaceTab;
  onWorkspaceTabChange: (tab: WorkspaceTab) => void;
  itemsSidebarQuery: string;
  onItemsSidebarQueryChange: (value: string) => void;
  itemsFunctionsOpen: boolean;
  onToggleItemsFunctions: () => void;
  itemsTablesOpen: boolean;
  onToggleItemsTables: () => void;
  openTableTabs: string[];
  onSelectTable: (tableName: string) => void;
  onCloseTableTab: (tableName: string) => void;
  onShowConnectionPicker: () => void;
  onShowDatabasePicker: () => void;
  onMinimizeWindow: () => void;
  onToggleMaximize: () => void;
  onCloseWindow: () => void;
}

export default function WorkspaceShell(props: WorkspaceShellProps) {
  const {
    workspace,
    workspaceTab,
    onWorkspaceTabChange,
    itemsSidebarQuery,
    onItemsSidebarQueryChange,
    itemsFunctionsOpen,
    onToggleItemsFunctions,
    itemsTablesOpen,
    onToggleItemsTables,
    openTableTabs,
    onSelectTable,
    onCloseTableTab,
    onShowConnectionPicker,
    onShowDatabasePicker,
    onMinimizeWindow,
    onToggleMaximize,
    onCloseWindow,
  } = props;

  // Close the active result tab with Ctrl+W, similar to desktop editors.
  useEffect(() => {
    if (!workspace.open || !workspace.connection) {
      return;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (!event.ctrlKey || (event.key !== "w" && event.key !== "W")) {
        return;
      }

      if (!workspace.selectedTable) {
        return;
      }

      event.preventDefault();
      onCloseTableTab(workspace.selectedTable);
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [workspace.open, workspace.connection, workspace.selectedTable, onCloseTableTab]);

  if (!workspace.open || !workspace.connection) return null;

  return (
    <div className="workspace-overlay" role="dialog" aria-modal="true">
      <div className="connections-shell">
        <header className="connections-shell__topbar" data-tauri-drag-region>
          <div className="connections-shell__topbar-section connections-shell__topbar-section--brand">
            <button
              type="button"
              className="connections-shell__icon-btn connections-shell__icon-btn--brand"
              aria-label="Open connection list"
              onClick={onShowConnectionPicker}
              data-tauri-drag-region="false"
            >
              <PlugZap size={16} />
            </button>
            <button
              type="button"
              className="connections-shell__icon-btn"
              aria-label="Open database picker"
              onClick={onShowDatabasePicker}
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
              onClick={onMinimizeWindow}
              data-tauri-drag-region="false"
            >
              <Minus size={14} />
            </button>

            <button
              type="button"
              className="connections-shell__icon-btn"
              aria-label="Toggle maximize"
              onClick={onToggleMaximize}
              data-tauri-drag-region="false"
            >
              <Square size={12} />
            </button>

            <button
              type="button"
              className="connections-shell__icon-btn"
              aria-label="Close"
              onClick={onCloseWindow}
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
                  onClick={() => onWorkspaceTabChange("items")}
                >
                  Items
                </button>
                <button
                  type="button"
                  className={`workspace-panel__tab${workspaceTab === "queries" ? " workspace-panel__tab--active" : ""}`}
                  onClick={() => onWorkspaceTabChange("queries")}
                >
                  Queries
                </button>
                <button
                  type="button"
                  className={`workspace-panel__tab${workspaceTab === "history" ? " workspace-panel__tab--active" : ""}`}
                  onClick={() => onWorkspaceTabChange("history")}
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
                  <ItemsSidebar
                    query={itemsSidebarQuery}
                    onQueryChange={onItemsSidebarQueryChange}
                    functionsOpen={itemsFunctionsOpen}
                    onToggleFunctions={onToggleItemsFunctions}
                    tablesOpen={itemsTablesOpen}
                    onToggleTables={onToggleItemsTables}
                    tablesLoading={workspace.tablesLoading}
                    tablesError={workspace.tablesError}
                    tables={workspace.tables}
                    selectedTable={workspace.selectedTable}
                    onSelectTable={onSelectTable}
                  />
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
                    {workspaceTab === "items" && (
                      <div className="workspace-panel__content workspace-panel__content--single">
                        <section className="workspace-panel__query">
                          {openTableTabs.length > 0 && (
                            <TableTabs
                              tabs={openTableTabs}
                              active={workspace.selectedTable}
                              onSelect={onSelectTable}
                              onClose={onCloseTableTab}
                            />
                          )}
                          <div className="workspace-panel__result">
                            <ResultGrid
                              result={workspace.queryResult}
                              running={workspace.queryRunning}
                              error={workspace.queryError}
                              tableName={workspace.selectedTable}
                            />
                          </div>
                        </section>
                      </div>
                    )}

                    {workspaceTab === "queries" && <div className="workspace-panel__empty-state" />}
                    {workspaceTab === "history" && <div className="workspace-panel__empty-state" />}
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
  );
}
