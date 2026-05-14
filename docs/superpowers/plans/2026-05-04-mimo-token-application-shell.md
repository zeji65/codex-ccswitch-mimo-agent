# MiMo Token Application Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a GitHub-ready documentation shell for a Codex + CC Switch multi-model AI R&D Agent system applying for MiMo Token Plan support.

**Architecture:** Keep the existing CC Switch codebase intact and add an application documentation package under `docs/application`. Add lightweight README entry points so reviewers can discover the application materials immediately.

**Tech Stack:** Markdown, Mermaid, YAML, existing CC Switch repository.

---

### Task 1: Add Application Documentation Entry Points

**Files:**
- Modify: `README.md`
- Modify: `README_ZH.md`
- Modify: `CHANGELOG.md`

- [x] **Step 1: Add root README links**

Add a short application package section near the top of `README.md`, linking to overview, architecture, Agent workflow, Token plan, and application summary.

- [x] **Step 2: Add Chinese README links**

Add the same application package section to `README_ZH.md` in Chinese.

- [x] **Step 3: Add changelog entry**

Add an `Unreleased` entry noting the new application documentation package.

### Task 2: Add Application Package

**Files:**
- Create: `docs/application/README.md`
- Create: `docs/application/architecture.md`
- Create: `docs/application/agents.md`
- Create: `docs/application/token-plan.md`
- Create: `docs/application/roadmap.md`
- Create: `docs/application/application-summary.md`
- Create: `docs/application/workflow-example.yaml`

- [x] **Step 1: Write project overview**

Create `docs/application/README.md` with positioning, pain points, solution, model split, pilot results, repository map, and next-step planning.

- [x] **Step 2: Write architecture document**

Create `docs/application/architecture.md` with the Codex entry, CC Switch gateway, semantic cache, multi-model routing, and verification loop.

- [x] **Step 3: Write Agent workflow document**

Create `docs/application/agents.md` describing the five specialized Agents, inputs, outputs, and collaboration standard.

- [x] **Step 4: Write Token plan**

Create `docs/application/token-plan.md` with current usage, target usage, MiMo workload, and capacity request.

- [x] **Step 5: Write roadmap and summary**

Create `docs/application/roadmap.md` and `docs/application/application-summary.md` for application review and follow-up development.

- [x] **Step 6: Add safe workflow example**

Create `docs/application/workflow-example.yaml` without secrets or company-private rules.

### Task 3: Verify Documentation Shell

**Files:**
- Inspect: `docs/application/*`
- Inspect: `README.md`
- Inspect: `README_ZH.md`
- Inspect: `CHANGELOG.md`

- [ ] **Step 1: Verify changed files**

Run: `git diff -- README.md README_ZH.md CHANGELOG.md docs/application docs/superpowers/plans/2026-05-04-mimo-token-application-shell.md`

- [ ] **Step 2: Verify no source files were modified by this task**

Run: `git status --short`

- [ ] **Step 3: Report outcome**

Summarize changed documentation, verification commands, residual risk, and whether GitHub push still needs credentials.
