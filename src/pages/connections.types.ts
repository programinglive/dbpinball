export interface DbConnection {
  id: string;
  name: string;
  engine: "mysql" | "postgres";
  host: string;
  port: number;
  database: string;
  username: string;
  useSsl: boolean;
}

export type QueryResultRow = (string | null)[];

export type QueryResultPayload = {
  columns: string[];
  rows: QueryResultRow[];
};

export type WorkspaceState = {
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

export const formatError = (err: unknown) => {
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

export const createInitialWorkspaceState = (): WorkspaceState => ({
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
