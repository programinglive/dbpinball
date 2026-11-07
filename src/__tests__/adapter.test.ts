import { describe, it, expect } from "vitest";
import packageJson from "../../package.json";

describe("dbpinball monorepo structure", () => {
  it("includes packages/* in workspace configuration", () => {
    expect(packageJson.workspaces).toContain("packages/*");
  });
});
