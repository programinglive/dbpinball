import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DbConnection, WorkspaceState, QueryResultPayload } from "@/pages/connections.types";
import { formatError } from "@/pages/connections.types";

interface UseWorkspaceQueryArgs {
  workspace: WorkspaceState;
  setWorkspace: React.Dispatch<React.SetStateAction<WorkspaceState>>;
  setOpenTableTabs: React.Dispatch<React.SetStateAction<string[]>>;
}

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

export function useWorkspaceQuery(args: UseWorkspaceQueryArgs) {
  const { workspace, setWorkspace, setOpenTableTabs } = args;

  const handleSelectDatabase = useCallback(
    async (dbName: string) => {
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
      setOpenTableTabs([]);

      try {
        const tables = await invoke<string[]>("list_tables_cmd", { conn, database: dbName });
        setWorkspace((prev) => {
          if (!prev.connection || prev.connection.id !== connectionId || prev.selectedDatabase !== dbName) {
            return prev;
          }

          const hasTables = tables.length > 0;
          const firstTable = hasTables ? tables[0] : null;
          const shouldResetQuery = !prev.queryDirty;
          const nextQueryText =
            firstTable && shouldResetQuery
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
    },
    [workspace.connection, setWorkspace, setOpenTableTabs]
  );

  const handleSelectTable = useCallback(
    (tableName: string) => {
      setOpenTableTabs((prev) => (prev.includes(tableName) ? prev : [...prev, tableName]));
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
          queryResult: shouldResetQuery ? null : prev.queryResult,
          queryError: null,
        };
      });

      setWorkspace((prev) => {
        if (!prev.connection || !prev.selectedDatabase) {
          return prev;
        }

        const sql = buildSelectQuery(prev.connection.engine, tableName, prev.queryLimit);

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
    },
    [setOpenTableTabs, setWorkspace]
  );

  const handleCloseTableTab = useCallback(
    (tableName: string) => {
      setOpenTableTabs((prevTabs) => {
        const remaining = prevTabs.filter((name) => name !== tableName);

        setWorkspace((prevWorkspace) => {
          if (prevWorkspace.selectedTable !== tableName) {
            return prevWorkspace;
          }

          const nextTable = remaining[remaining.length - 1] ?? null;

          if (!nextTable) {
            return {
              ...prevWorkspace,
              selectedTable: null,
              queryResult: null,
              queryError: null,
            };
          }

          // Reuse handleSelectTable for side effects; rely on its internal logic.
          handleSelectTable(nextTable);

          return prevWorkspace;
        });

        return remaining;
      });
    },
    [setOpenTableTabs, setWorkspace, handleSelectTable]
  );

  return {
    handleSelectDatabase,
    handleSelectTable,
    handleCloseTableTab,
  };
}
