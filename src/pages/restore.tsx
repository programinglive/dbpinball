import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Progress } from "@/components/ui/progress";
import { Input } from "@/components/ui/input";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Upload } from "lucide-react";

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

interface RestoreOptions {
  dropExisting: boolean;
  stopOnError: boolean;
}

export default function RestorePage() {
  const [connections, setConnections] = useState<DbConnection[]>([]);
  const [selectedConn, setSelectedConn] = useState<string>("");
  const [filePath, setFilePath] = useState<string>("");
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [result, setResult] = useState<string | null>(null);
  const [options, setOptions] = useState<RestoreOptions>({
    dropExisting: false,
    stopOnError: true,
  });

  useEffect(() => {
    loadConnections();
    const unlisten = listen<string>("job-progress", (e) => {
      setLogs((prev) => [...prev, e.payload]);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const loadConnections = () => {
    try {
      const stored = localStorage.getItem("dbpinball_connections");
      if (stored) {
        setConnections(JSON.parse(stored));
      }
    } catch (e) {
      console.error("Failed to load connections:", e);
    }
  };

  const handleStartRestore = async () => {
    if (!selectedConn || !filePath) {
      alert("Please select a connection and file");
      return;
    }

    const conn = connections.find((c) => c.id === selectedConn);
    if (!conn) return;

    setRunning(true);
    setLogs([]);
    setResult(null);

    try {
      const file: string = await invoke("restore", {
        conn,
        file: filePath,
        opts: options,
      });
      setResult(file);
      setLogs((prev) => [...prev, `✓ Restore completed successfully`]);
    } catch (e) {
      setLogs((prev) => [...prev, `✗ Error: ${e}`]);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-2xl font-bold">Restore Database</h2>
        <p className="text-sm text-muted-foreground">Restore a database from a backup file</p>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <div className="lg:col-span-1 space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Restore Settings</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div>
                <Label htmlFor="connection">Target Database</Label>
                <Select value={selectedConn} onValueChange={setSelectedConn} disabled={running}>
                  <SelectTrigger>
                    <SelectValue placeholder="Select a connection" />
                  </SelectTrigger>
                  <SelectContent>
                    {connections.map((conn) => (
                      <SelectItem key={conn.id} value={conn.id}>
                        {conn.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div>
                <Label htmlFor="file">Backup File Path</Label>
                <Input
                  id="file"
                  placeholder="/path/to/backup.sql"
                  value={filePath}
                  onChange={(e) => setFilePath(e.target.value)}
                  disabled={running}
                />
              </div>

              <div className="space-y-3">
                <Label>Options</Label>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="dropExisting"
                    checked={options.dropExisting}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, dropExisting: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="dropExisting" className="font-normal cursor-pointer">
                    Drop schema if exists
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="stopOnError"
                    checked={options.stopOnError}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, stopOnError: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="stopOnError" className="font-normal cursor-pointer">
                    Stop on first error
                  </Label>
                </div>
              </div>

              <Button
                onClick={handleStartRestore}
                disabled={running || !selectedConn || !filePath}
                className="w-full"
              >
                <Upload className="w-4 h-4 mr-2" />
                {running ? "Restoring..." : "Start Restore"}
              </Button>
            </CardContent>
          </Card>
        </div>

        <div className="lg:col-span-2">
          <Card>
            <CardHeader>
              <CardTitle>Progress & Logs</CardTitle>
              <CardDescription>Real-time restore progress</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              {running && <Progress value={undefined} className="w-full" />}

              <div className="h-64 rounded border p-3 overflow-auto bg-muted font-mono text-xs space-y-1">
                {logs.length === 0 ? (
                  <p className="text-muted-foreground">Waiting for restore to start...</p>
                ) : (
                  logs.map((log, i) => (
                    <div key={i} className="text-foreground">
                      {log}
                    </div>
                  ))
                )}
              </div>

              {result && (
                <div className="p-3 rounded bg-green-50 border border-green-200">
                  <p className="text-sm font-medium text-green-900">✓ Restore completed successfully</p>
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
