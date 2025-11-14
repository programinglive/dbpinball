import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type QueryResultRow = (string | null)[];

interface QueryResultPayload {
  columns: string[];
  rows: QueryResultRow[];
}

interface ResultGridProps {
  result: QueryResultPayload | null;
  running: boolean;
  error: string | null;
}

export default function ResultGrid({ result, running, error }: ResultGridProps) {
  if (error) {
    return (
      <p className="workspace-panel__error" role="alert">
        {error}
      </p>
    );
  }

  if (running && !result) {
    return <div className="workspace-panel__status">Running query</div>;
  }

  if (!result || result.columns.length === 0) {
    return null;
  }

  return (
    <div className="workspace-panel__result-scroll">
      <Table className="workspace-panel__result-table">
        <TableHeader>
          <TableRow>
            {result.columns.map((column, columnIndex) => (
              <TableHead key={`${column}-${columnIndex}`}>{column}</TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {result.rows.map((row, rowIndex) => (
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
    </div>
  );
}
