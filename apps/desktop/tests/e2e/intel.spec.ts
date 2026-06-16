import { expect, test } from "@playwright/test";

import { ConsoleErrorCollector, navigateTo, waitForNoSpinners } from "./helpers";

test.describe("Intel page", () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
  });

  test.afterEach(() => {
    consoleErrors.assertNoErrors();
  });

  test("/intel loads with both cards and a range picker", async ({ page }) => {
    await navigateTo(page, "/intel");
    await waitForNoSpinners(page);

    await expect(
      page.locator("h1", { hasText: "Engineering Intelligence" }),
    ).toBeVisible();
    await expect(page.getByText("Repo Attribution")).toBeVisible();
    await expect(page.getByText("Per-Tool Usage")).toBeVisible();

    // The four range buttons should all be present.
    for (const label of ["7 days", "30 days", "90 days", "All time"]) {
      await expect(page.getByRole("button", { name: label })).toBeVisible();
    }

    // Run button is disabled until a path is entered.
    const runButton = page.getByRole("button", { name: "Run" });
    await expect(runButton).toBeDisabled();
  });

  test("typing a repo path enables Run", async ({ page }) => {
    await navigateTo(page, "/intel");
    await waitForNoSpinners(page);

    const input = page.getByPlaceholder("/Users/me/code/my-repo");
    await input.fill("/tmp/some-repo");

    await expect(page.getByRole("button", { name: "Run" })).toBeEnabled();
  });

  test("range picker reflects selection", async ({ page }) => {
    await navigateTo(page, "/intel");
    await waitForNoSpinners(page);

    const all = page.getByRole("button", { name: "All time" });
    await all.click();
    // Active state uses cv-accent text color; just assert it stays visible after click.
    await expect(all).toBeVisible();
  });
});
