import "@testing-library/jest-dom/vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import ResultGrid from "../ResultGrid";

describe("ResultGrid layout", () => {
  const sampleResult = {
    columns: ["id", "name"],
    rows: [
      ["1", "Alice"],
      ["2", "Bob"],
    ],
  };

  it("renders the info bar and status bar for a selected table", () => {
    render(
      <ResultGrid result={sampleResult} running={false} error={null} tableName="activity_log" />
    );

    expect(screen.getByText("activity_log")).toBeInTheDocument();
    expect(screen.getByText("2 rows")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Data" })).toHaveClass(
      "workspace-panel__statusbar-seg--active"
    );
    const [actions] = screen.getAllByTestId("result-view-actions");
    expect(actions).toBeInTheDocument();
    expect(within(actions).getByText("Columns")).toBeInTheDocument();
  });

  it("switches between Data, Structure, and Raw views without breaking controls", async () => {
    const user = userEvent.setup();
    render(
      <ResultGrid result={sampleResult} running={false} error={null} tableName="stocks" />
    );

    const [viewToggle] = screen.getAllByTestId("result-view-toggle");

    await user.click(within(viewToggle).getByRole("button", { name: "Structure" }));
    const structureList = document.querySelector(
      ".workspace-panel__structure-list"
    ) as HTMLElement | null;
    expect(structureList).toBeInTheDocument();
    expect(within(structureList!).getByText("id")).toBeInTheDocument();

    await user.click(within(viewToggle).getByRole("button", { name: "Raw" }));
    const rawBlock = document.querySelector(".workspace-panel__raw");
    expect(rawBlock).toBeInTheDocument();

    await user.click(within(viewToggle).getByRole("button", { name: "Data" }));
    expect(screen.getAllByRole("table").length).toBeGreaterThan(0);
  });
});
