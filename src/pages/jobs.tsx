import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Trash2, Download, RotateCcw } from "lucide-react";

interface Job {
  id: string;
  type: "backup" | "restore" | "copy";
  database: string;
  status: "running" | "completed" | "failed";
  startTime: string;
  endTime?: string;
  artifactPath?: string;
  error?: string;
}

export default function JobsPage() {
  const [jobs, setJobs] = useState<Job[]>([]);

  useEffect(() => {
    loadJobs();
  }, []);

  const loadJobs = () => {
    try {
      const stored = localStorage.getItem("dbpinball_jobs");
      if (stored) {
        setJobs(JSON.parse(stored));
      }
    } catch (e) {
      console.error("Failed to load jobs:", e);
    }
  };

  const saveJobs = (updatedJobs: Job[]) => {
    localStorage.setItem("dbpinball_jobs", JSON.stringify(updatedJobs));
    setJobs(updatedJobs);
  };

  const handleDeleteJob = (id: string) => {
    if (confirm("Delete this job?")) {
      saveJobs(jobs.filter((j) => j.id !== id));
    }
  };

  const handleDownloadArtifact = (artifactPath: string) => {
    // In a real app, this would trigger a download
    alert(`Download: ${artifactPath}`);
  };

  const getStatusBadge = (status: string) => {
    switch (status) {
      case "running":
        return <Badge variant="outline">Running</Badge>;
      case "completed":
        return <Badge variant="default" className="bg-green-600">Completed</Badge>;
      case "failed":
        return <Badge variant="destructive">Failed</Badge>;
      default:
        return <Badge>{status}</Badge>;
    }
  };

  const getTypeLabel = (type: string) => {
    switch (type) {
      case "backup":
        return "Backup";
      case "restore":
        return "Restore";
      case "copy":
        return "Copy";
      default:
        return type;
    }
  };

  const formatTime = (dateString: string) => {
    return new Date(dateString).toLocaleString();
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-2xl font-bold">Job History</h2>
        <p className="text-sm text-muted-foreground">View and manage your backup, restore, and copy jobs</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Recent Jobs</CardTitle>
          <CardDescription>{jobs.length} job(s) total</CardDescription>
        </CardHeader>
        <CardContent>
          {jobs.length === 0 ? (
            <p className="text-center text-muted-foreground py-8">No jobs yet. Start a backup, restore, or copy operation.</p>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Type</TableHead>
                    <TableHead>Database</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Start Time</TableHead>
                    <TableHead>End Time</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {jobs.map((job) => (
                    <TableRow key={job.id}>
                      <TableCell className="font-medium">{getTypeLabel(job.type)}</TableCell>
                      <TableCell>{job.database}</TableCell>
                      <TableCell>{getStatusBadge(job.status)}</TableCell>
                      <TableCell className="text-sm">{formatTime(job.startTime)}</TableCell>
                      <TableCell className="text-sm">{job.endTime ? formatTime(job.endTime) : "-"}</TableCell>
                      <TableCell className="text-right space-x-2">
                        {job.artifactPath && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => handleDownloadArtifact(job.artifactPath!)}
                            title="Download artifact"
                          >
                            <Download className="w-4 h-4" />
                          </Button>
                        )}
                        {job.status === "completed" && (
                          <Button
                            variant="ghost"
                            size="sm"
                            title="Re-run job"
                          >
                            <RotateCcw className="w-4 h-4" />
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => handleDeleteJob(job.id)}
                          title="Delete job"
                        >
                          <Trash2 className="w-4 h-4" />
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
