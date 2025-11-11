import { BrowserRouter as Router } from "react-router-dom";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import ConnectionsPage from "@/pages/connections";
import BackupPage from "@/pages/backup";
import RestorePage from "@/pages/restore";
import CopyPage from "@/pages/copy";
import JobsPage from "@/pages/jobs";
import "./App.css";

function App() {
  return (
    <Router>
      <div className="min-h-screen bg-background">
        <div className="border-b">
          <div className="container mx-auto px-4 py-4">
            <h1 className="text-3xl font-bold">dbpinball</h1>
            <p className="text-sm text-muted-foreground">Database Backup, Restore & Copy Tool</p>
          </div>
        </div>

        <div className="container mx-auto px-4 py-8">
          <Tabs defaultValue="connections" className="w-full">
            <TabsList className="grid w-full grid-cols-5">
              <TabsTrigger value="connections">Connections</TabsTrigger>
              <TabsTrigger value="backup">Backup</TabsTrigger>
              <TabsTrigger value="restore">Restore</TabsTrigger>
              <TabsTrigger value="copy">Copy</TabsTrigger>
              <TabsTrigger value="jobs">Jobs</TabsTrigger>
            </TabsList>

            <TabsContent value="connections" className="mt-6">
              <ConnectionsPage />
            </TabsContent>

            <TabsContent value="backup" className="mt-6">
              <BackupPage />
            </TabsContent>

            <TabsContent value="restore" className="mt-6">
              <RestorePage />
            </TabsContent>

            <TabsContent value="copy" className="mt-6">
              <CopyPage />
            </TabsContent>

            <TabsContent value="jobs" className="mt-6">
              <JobsPage />
            </TabsContent>
          </Tabs>
        </div>
      </div>
    </Router>
  );
}

export default App;
