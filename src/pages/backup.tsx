import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Progress } from "@/components/ui/progress";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Download } from "lucide-react";

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

interface BackupOptions {
  structure: boolean;
  data: boolean;
  includeViews: boolean;
  includeRoutines: boolean;
  includeTriggers: boolean;
  bulkInsert: boolean;
  ignoreDefiner: boolean;
  compress: boolean;
}

export default function BackupPage() {
  const [connections, setConnections] = useState<DbConnection[]>([]);
  const [selectedConn, setSelectedConn] = useState<string>("");
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [result, setResult] = useState<string | null>(null);
  const [options, setOptions] = useState<BackupOptions>({
    structure: true,
    data: true,
    includeViews: true,
    includeRoutines: true,
    includeTriggers: true,
    bulkInsert: true,
    ignoreDefiner: true,
    compress: false,
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

  const handleStartBackup = async () => {
    if (!selectedConn) {
      alert("Please select a connection");
      return;
    }

    const conn = connections.find((c) => c.id === selectedConn);
    if (!conn) return;

    setRunning(true);
    setLogs([]);
    setResult(null);

    try {
      const file: string = await invoke("backup", {
        conn,
        opts: options,
      });
      setResult(file);
      setLogs((prev) => [...prev, `✓ Backup saved to: ${file}`]);
    } catch (e) {
      setLogs((prev) => [...prev, `✗ Error: ${e}`]);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-2xl font-bold">Backup Database</h2>
        <p className="text-sm text-muted-foreground">Create a backup of your database</p>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <div className="lg:col-span-1 space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Backup Settings</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div>
                <Label htmlFor="connection">Database Connection</Label>
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

              <div className="space-y-3">
                <Label>Content</Label>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="structure"
                    checked={options.structure}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, structure: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="structure" className="font-normal cursor-pointer">
                    Structure
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="data"
                    checked={options.data}
                    onCheckedChange={(checked) => setOptions({ ...options, data: checked as boolean })}
                    disabled={running}
                  />
                  <Label htmlFor="data" className="font-normal cursor-pointer">
                    Data
                  </Label>
                </div>
              </div>

              <div className="space-y-3">
                <Label>Include</Label>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="views"
                    checked={options.includeViews}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, includeViews: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="views" className="font-normal cursor-pointer">
                    Views
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="routines"
                    checked={options.includeRoutines}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, includeRoutines: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="routines" className="font-normal cursor-pointer">
                    Routines
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="triggers"
                    checked={options.includeTriggers}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, includeTriggers: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="triggers" className="font-normal cursor-pointer">
                    Triggers
                  </Label>
                </div>
              </div>

              <div className="space-y-3">
                <Label>Options</Label>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="compress"
                    checked={options.compress}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, compress: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="compress" className="font-normal cursor-pointer">
                    Compress (ZIP)
                  </Label>
                </div>
                <div className="flex items-center space-x-2">
                  <Checkbox
                    id="ignoreDefiner"
                    checked={options.ignoreDefiner}
                    onCheckedChange={(checked) =>
                      setOptions({ ...options, ignoreDefiner: checked as boolean })
                    }
                    disabled={running}
                  />
                  <Label htmlFor="ignoreDefiner" className="font-normal cursor-pointer">
                    Ignore DEFINER (MySQL)
                  </Label>
                </div>
              </div>

              <Button
                onClick={handleStartBackup}
                disabled={running || !selectedConn}
                className="w-full"
              >
                <Download className="w-4 h-4 mr-2" />
                {running ? "Backing up..." : "Start Backup"}
              </Button>
            </CardContent>
          </Card>
        </div>

        <div className="lg:col-span-2">
          <Card>
            <CardHeader>
              <CardTitle>Progress & Logs</CardTitle>
              <CardDescription>Real-time backup progress</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              {running && <Progress value={undefined} className="w-full" />}

              <div className="h-64 rounded border p-3 overflow-auto bg-muted font-mono text-xs space-y-1">
                {logs.length === 0 ? (
                  <p className="text-muted-foreground">Waiting for backup to start...</p>
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
                  <p className="text-sm font-medium text-green-900">✓ Backup completed successfully</p>
                  <p className="text-xs text-green-700 mt-1 break-all">{result}</p>
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
