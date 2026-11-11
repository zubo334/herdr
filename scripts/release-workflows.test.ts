import { describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join } from "node:path";

const load = (name: string): any =>
  Bun.YAML.parse(readFileSync(new URL(`../.github/workflows/${name}.yml`, import.meta.url), "utf8"));
const preview = load("preview");
const release = load("release");
const adminGate = release.jobs["validate-release-source"].steps[0];

describe("fork workflow boundaries", () => {
  test("all checked-in workflows are valid YAML", () => {
    const directory = new URL("../.github/workflows/", import.meta.url);
    for (const name of readdirSync(directory).filter(name => name.endsWith(".yml"))) {
      expect(Bun.YAML.parse(readFileSync(new URL(name, directory), "utf8"))).toBeObject();
    }
  });

  test("upstream publishing and distribution jobs do not run in forks", () => {
    for (const [workflow, job] of [
      ["distribution", "validate"],
      ["label-next-release-issues", "close"],
      ["website-deploy", "trigger"],
      ["nix", "flake-check"],
      ["windows-arm64", "installer"],
    ]) {
      expect(load(workflow).jobs[job].if).toBe("github.repository == 'herdrdev/herdr'");
    }
  });

  test("fork CI keeps platform checks while allowing upstream merge subjects", () => {
    const ci = load("ci");
    expect(ci.on.push.branches).toContain("master");
    expect(ci.on.pull_request).toBeDefined();
    expect(ci.jobs["conventional-commits"].if).toContain("github.repository == 'herdrdev/herdr'");
    expect(ci.jobs.check.if).toBeUndefined();
    expect(ci.jobs.check.strategy.matrix.include.map((entry: any) => entry.os))
      .toEqual(["ubuntu-latest", "macos-latest", "windows-latest"]);
    expect(ci.jobs["windows-conpty-package"].if).toBeUndefined();
    expect(ci.env.HERDR_USAGE_DISABLED).toBe("1");
  });

  test("merged upstream syncs dispatch release from an immutable merged-source tag", () => {
    const sync = load("fork-tag-sync");
    expect(sync.on).toEqual({ pull_request_target: { types: ["closed"] } });
    const job = sync.jobs["sync-tag"];
    expect(job.if).toContain("github.event.repository.fork == true");
    expect(job.if).toContain("github.event.pull_request.merged == true");
    expect(job.if).toContain("github.event.pull_request.head.repo.full_name == 'herdrdev/herdr'");
    expect(job.if).toContain("github.event.pull_request.base.ref == github.event.repository.default_branch");
    expect(job.steps[0].with.ref).toBe("${{ github.event.pull_request.merge_commit_sha }}");
    expect(job.permissions).toEqual({ contents: "write", actions: "write" });
    const extract = job.steps.find((step: any) => step.id === "tag");
    expect(extract.env.PR_TITLE).toBe("${{ github.event.pull_request.title }}");
    expect(extract.run).not.toContain("${{ github.event.pull_request.title }}");
    const dispatch = job.steps.at(-1);
    expect(dispatch.run).toContain('git tag "$FORK_TAG" "$MERGE_SHA"');
    expect(dispatch.run).toContain('git rev-parse "$FORK_TAG^{commit}"');
    expect(dispatch.run).toContain("gh workflow run fork-release.yml");
    expect(dispatch.run).toContain('--ref "$FORK_TAG"');
    expect(dispatch.run).toContain('-f "version=${TAG#v}"');
    expect(dispatch.env.GH_TOKEN).toBe("${{ github.token }}");
  });
});

describe("official publishing workflow boundaries", () => {
  test("publishing is tag-only while normal PR CI remains enabled", () => {
    expect(preview.on).toEqual({ push: { tags: ["preview-*"] } });
    expect(release.on).toEqual({ push: { tags: ["v*"] } });
    expect(load("ci").on.pull_request).toBeDefined();
  });

  test("preview checks do not require a workstation Windows SDK", () => {
    const checks = preview.jobs.preflight.steps.find((step: any) => step.name === "Run checks");
    expect(checks.run.trim().split("\n")).toEqual(["just ci", "just docs-contract-test"]);
    expect(preview.jobs.build.strategy.matrix.include).toContainEqual({
      target: "x86_64-pc-windows-msvc",
      os: "windows-latest",
      name: "herdr-windows-x86_64.zip",
    });
    expect(preview.jobs.publish.needs).toContain("build");
  });

  test("each publishing job rechecks both actors before using credentials", () => {
    for (const [workflow, names] of [
      [preview, ["preflight", "publish"]],
      [release, ["validate-release-source", "release", "update-nix-package", "close-released-issues", "update-latest-json"]],
    ] as const) {
      for (const name of names) {
        const job = workflow.jobs[name];
        expect(job.if).toContain("github.event_name == 'push'");
        expect(job.if).toContain("startsWith(github.ref, 'refs/tags/");
        expect(job.steps[0]).toEqual(adminGate);
      }
    }
    expect(adminGate.run).toContain('"$GITHUB_ACTOR" "$GITHUB_TRIGGERING_ACTOR"');
    expect(adminGate.env.GH_TOKEN).toBe("${{ github.token }}");
    expect(adminGate.run).not.toContain("ogulcancelik");
  });

  test("release arguments are not interpolated into executable shell text", () => {
    const input = `untrusted'\"$(echo unexpected-command)`;
    for (const args of [
      ["preview", input],
      ["release-prepare", input, input],
      ["release-publish", input, input],
      ["release", input, input],
    ]) {
      const result = spawnSync("just", ["--dry-run", ...args], { encoding: "utf8" });
      expect(result.status).toBe(0);
      expect(result.stdout + result.stderr).not.toContain(input);
      expect(result.stdout + result.stderr).not.toContain("unexpected-command");
    }
  });

  test.skipIf(process.platform === "win32")("admin gate permits admins and fails closed for other roles or API errors", () => {
    const dir = mkdtempSync("/var/tmp/herdr-admin-gate-");
    try {
      writeFileSync(join(dir, "gh"), `#!/bin/sh
case "$2" in
  */collaborators/admin-*/permission) echo admin ;;
  */collaborators/maintainer/permission) echo maintain ;;
  */collaborators/writer/permission) echo write ;;
  *) exit 1 ;;
esac
`, { mode: 0o755 });
      for (const [actor, trigger, succeeds] of [
        ["admin-one", "admin-two", true],
        ["writer", "admin-two", false],
        ["admin-one", "writer", false],
        ["admin-one", "maintainer", false],
        ["admin-one", "api-error", false],
      ] as const) {
        const result = spawnSync("bash", ["-c", adminGate.run], {
          env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, GITHUB_REPOSITORY: "example/test", GITHUB_ACTOR: actor, GITHUB_TRIGGERING_ACTOR: trigger },
          encoding: "utf8",
        });
        expect(result.status === 0).toBe(succeeds);
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
