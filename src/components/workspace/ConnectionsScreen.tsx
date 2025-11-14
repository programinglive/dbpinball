import type { DbConnection } from "@/pages/connections.types";
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import ConnectionContextMenu from "@/components/workspace/ConnectionContextMenu";

interface ActiveContextMenu {
  connection: DbConnection;
  position: { x: number; y: number };
}

interface ConnectionsScreenProps {
  connections: DbConnection[];
  selectedId: string | null;
  testResult: Record<string, boolean>;
  activeContextMenu: ActiveContextMenu | null;
  onSelect: (id: string | null) => void;
  onRootContextMenu: (event: React.MouseEvent<HTMLDivElement>) => void;
  onRowContextMenu: (event: React.MouseEvent<HTMLDivElement>, conn: DbConnection) => void;
  onConnect: (conn: DbConnection) => void;
  onEdit: (conn: DbConnection) => void;
  onDelete: (conn: DbConnection) => void;
}

export default function ConnectionsScreen(props: ConnectionsScreenProps) {
  const {
    connections,
    selectedId,
    testResult,
    activeContextMenu,
    onSelect,
    onRootContextMenu,
    onRowContextMenu,
    onConnect,
    onEdit,
    onDelete,
  } = props;

  return (
    <div className="connections-screen">
      <div className="connections-list" onContextMenu={onRootContextMenu}>
        {connections.length === 0 ? (
          <div className="connections-list__empty">
            <p>No connections yet.</p>
            <p className="connections-list__empty-sub">Create a connection to get started.</p>
          </div>
        ) : (
          connections.map((conn) => {
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
                onClick={() => onSelect(conn.id)}
                onDoubleClick={() => onConnect(conn)}
                onContextMenu={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  onSelect(conn.id);
                  onRowContextMenu(event, conn);
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
        <ConnectionContextMenu
          open={true}
          position={activeContextMenu.position}
          onConnect={() => onConnect(activeContextMenu.connection)}
          onEdit={() => onEdit(activeContextMenu.connection)}
          onDelete={() => onDelete(activeContextMenu.connection)}
        />
      )}
    </div>
  );
}
