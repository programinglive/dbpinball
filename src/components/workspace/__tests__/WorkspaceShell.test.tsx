import "@testing-library/jest-dom/vitest";
import { describe, it, expect, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import WorkspaceShell from "@/components/workspace/WorkspaceShell";
import type { WorkspaceState } from "@/pages/connections.types";

const baseWorkspace: WorkspaceState = {
  open: true,
  loading: false,
  connection: {
    id: "conn_1",
    name: "Local",
    engine: "mysql",
    host: "localhost",
    port: 3306,
    database: "test",
    username: "root",
    useSsl: false,
  },
  databases: ["test"],
  tables: ["adjust_redemptions", "application_settings"],
  selectedDatabase: "test",
  selectedTable: "adjust_redemptions",
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
};

describe("WorkspaceShell keyboard shortcuts", () => {
  it("closes the active table tab when Ctrl+W is pressed", () => {
    const onWorkspaceTabChange = vi.fn();
    const onItemsSidebarQueryChange = vi.fn();
    const onToggleItemsFunctions = vi.fn();
    const onToggleItemsTables = vi.fn();
    const onSelectTable = vi.fn();
    const onCloseTableTab = vi.fn();
    const onShowConnectionPicker = vi.fn();
    const onShowDatabasePicker = vi.fn();
    const onMinimizeWindow = vi.fn();
    const onToggleMaximize = vi.fn();
    const onCloseWindow = vi.fn();

    render(
      <WorkspaceShell
        workspace={baseWorkspace}
        workspaceTab="items"
        onWorkspaceTabChange={onWorkspaceTabChange}
        itemsSidebarQuery=""
        onItemsSidebarQueryChange={onItemsSidebarQueryChange}
        itemsFunctionsOpen={true}
        onToggleItemsFunctions={onToggleItemsFunctions}
        itemsTablesOpen={true}
        onToggleItemsTables={onToggleItemsTables}
        openTableTabs={baseWorkspace.tables}
        onSelectTable={onSelectTable}
        onCloseTableTab={onCloseTableTab}
        onShowConnectionPicker={onShowConnectionPicker}
        onShowDatabasePicker={onShowDatabasePicker}
        onMinimizeWindow={onMinimizeWindow}
        onToggleMaximize={onToggleMaximize}
        onCloseWindow={onCloseWindow}
      />
    );

    // Simulate Ctrl+W at window level
    fireEvent.keyDown(window, { key: "w", ctrlKey: true });

    expect(onCloseTableTab).toHaveBeenCalledWith("adjust_redemptions");
  });
});
