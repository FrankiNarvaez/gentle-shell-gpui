// Gentle Shell GPUI: generative UI. The `gentle_ui` tool shows an interactive
// surface in the desktop app's conversation: a diagram, table, form, checklist,
// kanban board or chart the user edits natively, or an html page (an MCP App)
// run in an isolated web view. The app sends what the user does back with
// `/gentle-ui:event {json}` and the extension answers each event through the
// `gentle-ui.outcome` widget. State lives only in the session (tool result
// details and gentle-ui.event messages), so branches, forks and resumes
// rebuild it. Written by the app; edits here are overwritten.
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { StringEnum } from "@earendil-works/pi-ai";
import { Type } from "typebox";

const TOOL = "gentle_ui";
const EVENT_MESSAGE = "gentle-ui.event";
const EVENT_COMMAND = "gentle-ui:event";
const OUTCOME_WIDGET = "gentle-ui.outcome";
// Model context an html page shares: held until the next turn starts.
const CONTEXT_MESSAGE = "gentle-ui.context";

const NODE_KINDS = ["service", "database", "queue", "cache", "client", "external", "storage", "function", "note"];
const STATUSES = ["planned", "existing", "deprecated", "risk"];
const EDGE_STYLES = ["sync", "async", "data", "dependency"];
const MAX = { nodes: 200, edges: 400, groups: 50, id: 64, label: 80, text: 1000, errors: 10, listed: 60 };
const SLUG = /^[a-z0-9][a-z0-9-_]{0,63}$/;
// The session's cwd: absolute code refs inside it are stored relative to it.
let workspaceRoot = "";

type Json = Record<string, unknown>;
interface Group { id: string; label: string; description?: string }
interface Node {
	id: string; label: string; kind: string; group?: string; description?: string;
	tech?: string; status?: string; x?: number; y?: number; ref?: Json;
}
interface Edge { id: string; from: string; to: string; label?: string; style: string }
interface Spec { direction: "LR" | "TB"; groups: Group[]; nodes: Node[]; edges: Edge[] }
interface Surface { id: string; version: number; kind: string; title: string; spec: Json }
type Details = Surface | { id: string; version: number; kind: string; title: string; spec: null; closed: true };

/** A code reference `{path, line?, endLine?}` (or `"path:line[-end]"`) for an
 * item that corresponds to code: `/` separators, `.`/`..` folded, absolute
 * paths inside the workspace made relative. A path leaving the workspace
 * is a hard error (reported through `fail`, like the app's check). */
function normalizeRef(value: unknown, where: string, fail: (message: string) => unknown): Json | undefined {
	if (value === undefined || value === null) return undefined;
	let path: unknown;
	let line: unknown;
	let endLine: unknown;
	if (typeof value === "string") {
		const match = /^(.+?):([1-9]\d*)(?:-(\d+))?$/.exec(value.trim());
		[path, line, endLine] = match ? [match[1], Number(match[2]), match[3] === undefined ? undefined : Number(match[3])] : [value, undefined, undefined];
	} else if (isObject(value)) {
		[path, line, endLine] = [value.path, value.line, value.endLine ?? value.end_line];
	} else {
		fail(`${where}: ref must be an object {path, line?, endLine?} or "path:line"`);
		return undefined;
	}
	if (typeof path !== "string" || path.trim() === "") {
		fail(`${where}: ref needs a path`);
		return undefined;
	}
	let text = path.trim().replace(/\\/g, "/").replace(/^file:\/\//, "");
	const root = workspaceRoot.replace(/\\/g, "/").replace(/\/+$/, "");
	if (root !== "" && text.startsWith(`${root}/`)) text = text.slice(root.length + 1);
	const drive = /^[A-Za-z]:\//.exec(text)?.[0];
	const absolute = text.startsWith("/") || drive !== undefined;
	if (absolute && root !== "") {
		fail(`${where}: ref path "${cut(text, 80)}" is outside the workspace`);
		return undefined;
	}
	const prefix = text.startsWith("/") ? "/" : (drive ?? "");
	const parts: string[] = [];
	for (const part of text.slice(prefix.length).split("/")) {
		if (part === "" || part === ".") continue;
		if (part !== "..") parts.push(part);
		else if (parts.pop() === undefined && !absolute) {
			fail(`${where}: ref path "${cut(text, 80)}" escapes the workspace`);
			return undefined;
		}
	}
	if (parts.length === 0) {
		fail(`${where}: ref path "${cut(text, 80)}" names no file`);
		return undefined;
	}
	const [first, last] = [line, endLine].map((n) => (typeof n === "number" && Number.isInteger(n) && n >= 1 ? n : undefined));
	return { path: prefix + parts.join("/"), line: first, endLine: first !== undefined && last !== undefined && last > first ? last : undefined };
}

const isObject = (value: unknown): value is Json =>
	typeof value === "object" && value !== null && !Array.isArray(value);

/** `text` cut to `max` characters, ending in "…" when cut. */
function cut(text: string, max: number): string {
	const chars = Array.from(text);
	return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text;
}

function optionalText(value: unknown, max: number): string | undefined {
	return typeof value === "string" && value.trim() !== "" ? cut(value, max) : undefined;
}

/** A diagram spec with defaults filled and edge ids assigned; throws the
 * readable hard errors (the same ones the app checks). */
function normalize(raw: unknown): Spec {
	const errors: string[] = [];
	const fail = (message: string) => {
		if (errors.length < MAX.errors) errors.push(message);
	};
	if (!isObject(raw)) throw new Error("spec must be an object with a nodes array");
	if (!Array.isArray(raw.nodes)) throw new Error("spec.nodes must be an array");
	const list = (key: "groups" | "edges"): unknown[] => {
		const value = raw[key];
		if (value === undefined || value === null) return [];
		if (Array.isArray(value)) return value;
		fail(`spec.${key} must be an array`);
		return [];
	};
	const rawGroups = list("groups");
	const rawEdges = list("edges");
	if (raw.nodes.length > MAX.nodes) fail(`too many nodes (${raw.nodes.length}, at most ${MAX.nodes})`);
	if (rawEdges.length > MAX.edges) fail(`too many edges (${rawEdges.length}, at most ${MAX.edges})`);
	if (rawGroups.length > MAX.groups) fail(`too many groups (${rawGroups.length}, at most ${MAX.groups})`);
	const idOf = (item: unknown, what: string, index: number): string | undefined => {
		const id = isObject(item) && typeof item.id === "string" ? item.id.trim() : "";
		if (id === "") return void fail(`${what} ${index + 1} has no id`);
		if (id.length > MAX.id) return void fail(`${what} id "${cut(id, 20)}" is longer than ${MAX.id} characters`);
		return id;
	};

	const groups: Group[] = [];
	const groupIds = new Set<string>();
	rawGroups.forEach((item, index) => {
		const id = idOf(item, "group", index);
		if (id === undefined || !isObject(item)) return;
		if (groupIds.has(id)) return fail(`duplicate group id "${id}"`);
		groupIds.add(id);
		groups.push({ id, label: optionalText(item.label, MAX.label) ?? id, description: optionalText(item.description, MAX.text) });
	});

	const nodes: Node[] = [];
	const nodeIds = new Set<string>();
	raw.nodes.forEach((item, index) => {
		const id = idOf(item, "node", index);
		if (id === undefined || !isObject(item)) return;
		if (nodeIds.has(id)) return fail(`duplicate node id "${id}"`);
		nodeIds.add(id);
		const group = typeof item.group === "string" && item.group !== "" ? item.group : undefined;
		if (group !== undefined && !groupIds.has(group)) fail(`node "${id}" is in group "${group}", which is not in groups`);
		const kind = typeof item.kind === "string" ? item.kind.toLowerCase() : "";
		const status = typeof item.status === "string" ? item.status.toLowerCase() : "";
		const placed = Number.isFinite(item.x) && Number.isFinite(item.y);
		nodes.push({
			id,
			label: optionalText(item.label, MAX.label) ?? id,
			kind: NODE_KINDS.includes(kind) ? kind : "service",
			group,
			description: optionalText(item.description, MAX.text),
			tech: optionalText(item.tech, MAX.label),
			status: STATUSES.includes(status) ? status : undefined,
			x: placed ? Math.round(item.x as number) : undefined,
			y: placed ? Math.round(item.y as number) : undefined,
			ref: normalizeRef(item.ref, `node "${id}"`, fail),
		});
	});

	const edges: Edge[] = [];
	// Explicit ids first, so a generated one never takes an id a later edge names.
	const edgeIds = new Set(rawEdges.flatMap((e) => (isObject(e) && typeof e.id === "string" && e.id.trim() ? [e.id.trim()] : [])));
	const seenEdges = new Set<string>();
	rawEdges.forEach((item, index) => {
		if (!isObject(item)) return fail(`edge ${index + 1} must be an object`);
		const from = typeof item.from === "string" ? item.from : "";
		const to = typeof item.to === "string" ? item.to : "";
		if (!nodeIds.has(from)) return fail(`edge ${index + 1} starts at "${from}", which is not a node`);
		if (!nodeIds.has(to)) return fail(`edge ${index + 1} ends at "${to}", which is not a node`);
		let id = typeof item.id === "string" ? item.id.trim() : "";
		if (id === "") {
			id = `${from}->${to}`;
			for (let n = 2; edgeIds.has(id); n += 1) id = `${from}->${to}#${n}`;
			edgeIds.add(id);
		}
		if (seenEdges.has(id)) return fail(`duplicate edge id "${id}"`);
		seenEdges.add(id);
		const style = typeof item.style === "string" ? item.style.toLowerCase() : "";
		edges.push({ id, from, to, label: optionalText(item.label, MAX.label), style: EDGE_STYLES.includes(style) ? style : "sync" });
	});

	if (errors.length > 0) throw new Error(`invalid diagram spec:\n- ${errors.join("\n- ")}`);
	const direction = typeof raw.direction === "string" && raw.direction.toUpperCase() === "TB" ? "TB" : "LR";
	// JSON round trip drops the undefined fields.
	return JSON.parse(JSON.stringify({ direction, groups, nodes, edges })) as Spec;
}

/** Merge `item` into `target`: null clears a field, the id stays. */
function merge(target: Json, item: Json): void {
	for (const [key, value] of Object.entries(item)) {
		if (key === "id") continue;
		if (value === null) delete target[key];
		else target[key] = value;
	}
}

/** `spec` with `patch` applied (see the tool description), validated; `spec`
 * itself when there is no patch. */
function applyPatch(spec: Spec, patch: unknown): Spec {
	if (patch === undefined) return spec;
	if (!isObject(patch)) throw new Error("patch must be an object");
	const next = JSON.parse(JSON.stringify(spec)) as { direction: string; groups: Json[]; nodes: Json[]; edges: Json[] };
	const ids = (value: unknown): Set<string> =>
		new Set(Array.isArray(value) ? value.filter((id): id is string => typeof id === "string") : []);
	const items = (key: string): Json[] => {
		const value = patch[key];
		if (value === undefined || value === null) return [];
		if (!Array.isArray(value) || !value.every(isObject)) throw new Error(`patch.${key} must be an array of objects`);
		return value;
	};
	const remove = isObject(patch.remove) ? patch.remove : {};
	const goneNodes = ids(remove.nodes);
	const goneEdges = ids(remove.edges);
	const goneGroups = ids(remove.groups);
	next.nodes = next.nodes.filter((n) => !goneNodes.has(n.id as string));
	next.edges = next.edges.filter((e) => !goneEdges.has(e.id as string) && !goneNodes.has(e.from as string) && !goneNodes.has(e.to as string));
	next.groups = next.groups.filter((g) => !goneGroups.has(g.id as string));
	for (const node of next.nodes) if (goneGroups.has(node.group as string)) delete node.group;

	for (const key of ["groups", "nodes"] as const) {
		for (const item of items(key)) {
			if (typeof item.id !== "string" || item.id.trim() === "") throw new Error(`every item in patch.${key} needs an id`);
			const found = next[key].find((existing) => existing.id === item.id);
			if (found) merge(found, item);
			else next[key].push({ ...item });
		}
	}
	for (const item of items("edges")) {
		const found = typeof item.id === "string" && item.id !== ""
			? next.edges.find((e) => e.id === item.id)
			: next.edges.find((e) => e.from === item.from && e.to === item.to);
		if (found) merge(found, item);
		else next.edges.push({ ...item });
	}
	if (typeof patch.direction === "string") next.direction = patch.direction;
	return normalize(next);
}

function plural(count: number, word: string): string {
	return `${count} ${word}${count === 1 ? "" : "s"}`;
}

function listed(label: string, items: string[]): string {
	const shown = items.slice(0, MAX.listed).join(", ");
	const more = items.length > MAX.listed ? `, … and ${items.length - MAX.listed} more` : "";
	return `${label}: ${shown}${more}`;
}

/** What the model sees of a surface. */
function summary(surface: Surface): string {
	const spec = surface.spec as unknown as Spec;
	const lines = [
		`Diagram "${surface.title}" (${surface.id}, v${surface.version}) is shown to the user ` +
			`(${plural(spec.nodes.length, "node")}, ${plural(spec.edges.length, "edge")}, ${plural(spec.groups.length, "group")}, ${spec.direction}).`,
	];
	if (spec.groups.length > 0) lines.push(listed("Groups", spec.groups.map((g) => `${g.id} "${g.label}"`)));
	if (spec.nodes.length > 0) {
		lines.push(listed("Nodes", spec.nodes.map((n) => `${n.id} "${n.label}" (${[n.kind, n.group, n.status].filter(Boolean).join(", ")})`)));
	}
	if (spec.edges.length > 0) {
		lines.push(listed("Edges", spec.edges.map((e) =>
			`${e.from} → ${e.to}${e.label ? ` "${e.label}"` : ""}${e.style === "sync" ? "" : ` [${e.style}]`}`)));
	}
	lines.push(
		`The user can move, rename, connect, group and delete parts, change a node's kind, status, tech and description, and flip the direction; their edits arrive as "${EVENT_MESSAGE}" messages.`,
	);
	return lines.join("\n");
}

// ── Native kinds: table, form, checklist, kanban, chart ──────────────────────
// Each mirrors `gentle_core::genui::<kind>` (same defaults and hard errors).

const LIMITS = {
	columns: 30, rows: 500, cell: 500, fields: 50, options: 50, items: 200, itemLabel: 200,
	boardColumns: 12, cards: 300, tags: 8, tag: 40, cardTitle: 120, series: 12, points: 1000,
	summaryRows: 30, summaryLines: 80,
};
const COLUMN_TYPES = ["text", "number", "date", "status", "link"];
const ALIGNS = ["left", "center", "right"];
const FIELD_TYPES = ["text", "textarea", "number", "select", "multiselect", "checkbox", "radio", "slider", "date"];
const CHART_TYPES = ["bar", "line", "area", "pie", "donut", "scatter"];
// Live data (`live` on tables, charts and checklists; `gentle_core::genui::live`):
// the app reads the source and fills the spec, so the agent doesn't push rows.
const LIVE_SOURCES = ["jobs", "subagents", "git:status", "git:log", "todo", "command"];
const PUSHED_SOURCES = ["jobs", "subagents", "todo"];
const LIVE = { minInterval: 5, maxInterval: 3600, gitInterval: 10, commandInterval: 30, command: 500, path: 200 };
const RUN_STATUSES = ["pending", "running", "done", "failed", "skipped"];
const RUN_MODES = ["idle", "all", "step"];

/** A JSONPath-lite pick, checked as `live::Path::parse` does: `$`, `.key`,
 * `["key"]`, `[0]`, `[*]`, `.*`; without `$` it starts at the record. */
function checkPath(text: string): string {
	const path = text.trim();
	if (path === "") throw new Error("a path can't be empty");
	if (Array.from(path).length > LIVE.path) throw new Error(`bad path "${cut(path, 40)}": longer than ${LIVE.path} characters`);
	let rest = path.startsWith("$") ? path.slice(1) : path.startsWith("[") ? path : `.${path}`;
	const segment = /^(?:\.\*|\.[^.[\]"']+|\[\s*(?:\*|\d+)\s*\]|\["[^"]*"\]|\['[^']*'\])/;
	while (rest !== "") {
		const match = segment.exec(rest);
		if (!match) throw new Error(`bad path "${cut(path, 40)}"`);
		rest = rest.slice(match[0].length);
	}
	return path;
}

/** `live` checked and with defaults (as `live::Live::parse`); undefined when absent. */
function normalizeLive(raw: unknown, fail: (message: string) => undefined): Json | undefined {
	if (raw === undefined || raw === null) return undefined;
	const object = isObject(raw) ? raw : undefined;
	if (typeof raw !== "string" && object === undefined) return fail("live must be an object or a source name");
	const named = typeof raw === "string" ? raw : typeof object?.source === "string" ? object.source : undefined;
	const hasCommand = object?.command !== undefined && object?.command !== null;
	const source = named !== undefined ? named.trim().toLowerCase() : hasCommand ? "command" : undefined;
	if (source === undefined) return fail("live needs a source");
	if (!LIVE_SOURCES.includes(source)) return fail(`unknown live source "${cut(String(named), 40)}" (one of ${LIVE_SOURCES.join(", ")})`);
	const live: Json = { source };
	if (source === "command") {
		const c = object?.command;
		if (!hasCommand) return fail("a command source needs live.command: {cmd, cwd?, parse?}");
		const given = typeof c === "string" ? { cmd: c } : isObject(c) ? c : undefined;
		if (given === undefined) return fail("live.command must be {cmd, cwd?, parse?}");
		const cmd = typeof given.cmd === "string" ? given.cmd.trim() : "";
		if (cmd === "") return fail("live.command.cmd must be a non-empty string");
		if (Array.from(cmd).length > LIVE.command) return fail(`live.command.cmd is longer than ${LIVE.command} characters`);
		if (/[\u0000-\u001f\u007f-\u009f]/.test(cmd)) return fail("live.command.cmd must be one line");
		const command: Json = { cmd };
		if (given.cwd !== undefined && given.cwd !== null) {
			if (typeof given.cwd !== "string") return fail("live.command.cwd must be a folder path");
			const dir = given.cwd.trim().replace(/[/\\]+$/, "");
			if (dir !== "" && dir !== ".") {
				if (/^[/\\~]/.test(dir) || dir[1] === ":" || dir.split(/[/\\]/).includes("..")) {
					return fail(`live.command.cwd "${cut(dir, 60)}" must be a folder inside the workspace (relative, without "..")`);
				}
				command.cwd = dir;
			}
		}
		const parse = given.parse === undefined || given.parse === null ? "lines" : lower(given.parse);
		if (!["json", "lines", "csv"].includes(parse)) return fail("live.command.parse must be json, lines or csv");
		command.parse = parse;
		live.command = command;
	} else if (hasCommand) {
		return fail(`live.command only goes with source "command", not "${source}"`);
	}
	if (!PUSHED_SOURCES.includes(source)) {
		const given = num(object?.interval ?? (isObject(object?.command) ? object.command.interval : undefined));
		live.interval = given === undefined
			? (source === "command" ? LIVE.commandInterval : LIVE.gitInterval)
			: Math.min(LIVE.maxInterval, Math.max(LIVE.minInterval, Math.round(Math.max(0, given))));
	}
	const rawMap = object?.map;
	if (rawMap !== undefined && rawMap !== null) {
		if (!isObject(rawMap)) return fail("live.map must be an object");
		const map: Json = {};
		const pick = (where: string, value: unknown): string | undefined => {
			if (typeof value !== "string") return fail(`live.map.${where} must be a path string`);
			try {
				return checkPath(value);
			} catch (error) {
				return fail(`live.map.${where}: ${(error as Error).message}`);
			}
		};
		for (const key of ["items", "id", "label", "status", "note"]) {
			if (rawMap[key] !== undefined) map[key] = pick(key, rawMap[key]);
		}
		for (const key of ["columns", "series"]) {
			const picks = rawMap[key];
			if (picks === undefined) continue;
			if (!isObject(picks)) {
				fail(`live.map.${key} must be an object of paths`);
				continue;
			}
			map[key] = Object.fromEntries(Object.entries(picks).flatMap(([name, path]) => {
				const checked = pick(`${key}.${name}`, path);
				return checked === undefined ? [] : [[name, checked]];
			}));
		}
		if (Object.keys(map).length > 0) live.map = map;
	}
	return live;
}

/** The summary's line about a live spec: the app refreshes it. */
function withLive(text: string, surface: Surface): string {
	const live = surface.spec.live;
	if (!isObject(live)) return text;
	const from = live.source === "command" && isObject(live.command)
		? `the command \`${String(live.command.cmd)}\` (run only if the user allows it)`
		: `the app's ${String(live.source)} data`;
	const every = typeof live.interval === "number" ? `, every ${live.interval} s` : "";
	return `${text}\nLive: the app fills it from ${from}${every}; don't update its rows, values or items yourself.`;
}

/** Hard errors, at most MAX.errors; `check` throws them all at once. */
function problems(noun: string) {
	const errors: string[] = [];
	return {
		fail: (message: string): undefined => {
			if (errors.length < MAX.errors) errors.push(message);
			return undefined;
		},
		check: () => {
			if (errors.length > 0) throw new Error(`invalid ${noun} spec:\n- ${errors.join("\n- ")}`);
		},
	};
}

/** A trimmed non-empty string or a number, cut to MAX.id. */
function idText(value: unknown): string | undefined {
	const id = typeof value === "number" ? String(value) : typeof value === "string" ? value.trim() : "";
	return id === "" ? undefined : Array.from(id).slice(0, MAX.id).join("");
}

/** A finite number, or a numeric string ("1,200" included). */
function num(value: unknown): number | undefined {
	const n = typeof value === "number" ? value : typeof value === "string" && value.trim() !== "" ? Number(value.replace(/,/g, "")) : NaN;
	return Number.isFinite(n) ? n : undefined;
}

const lower = (value: unknown): string => (typeof value === "string" ? value.trim().toLowerCase() : "");
const list = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

/** A slug of `label` (`fallback` when it has none), "-2", "-3"… until free. */
function slugOf(label: string, fallback: string, taken: (id: string) => boolean): string {
	const slug = Array.from(label.toLowerCase().replace(/[^\p{L}\p{N}_]+/gu, "-").replace(/^-+|-+$/g, "")).slice(0, MAX.id - 4).join("") || fallback;
	if (!taken(slug)) return slug;
	let n = 2;
	while (taken(`${slug}-${n}`)) n += 1;
	return `${slug}-${n}`;
}

/** Upsert `items` into `target` by `key`: found ones merge (null clears a
 * field), new ones are appended. */
function upsert(target: Json[], items: unknown, key: string, where: string, mergeOne?: (found: Json, item: Json) => void): void {
	if (items === undefined || items === null) return;
	if (!Array.isArray(items) || !items.every(isObject)) throw new Error(`patch.${where} must be an array of objects`);
	for (const item of items) {
		const id = idText(item[key]);
		if (id === undefined) throw new Error(`every item in patch.${where} needs a ${key}`);
		const found = target.find((existing) => idText(existing[key]) === id);
		if (!found) target.push({ ...item });
		else if (mergeOne) mergeOne(found, item);
		else for (const [k, v] of Object.entries(item)) if (k !== key) v === null ? delete found[k] : (found[k] = v);
	}
}

/** Top-level `keys` of `patch` copied onto `next` (null clears). */
function scalars(next: Json, patch: Json, keys: string[]): void {
	for (const key of keys) if (key in patch) patch[key] === null ? delete next[key] : (next[key] = patch[key]);
}

const removed = (patch: Json, key: string): Set<string> =>
	new Set(list(isObject(patch.remove) ? patch.remove[key] : undefined).flatMap((id) => idText(id) ?? []));
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
const quoted = (text: unknown, max = 40): string => `"${cut(String(text ?? ""), max)}"`;

function cellText(value: unknown): string {
	if (value === null || value === undefined) return "";
	if (typeof value === "boolean") return value ? "yes" : "no";
	if (isObject(value)) return String(value.text ?? value.url ?? "");
	return String(value);
}

function cleanCell(value: unknown): unknown {
	if (value === null || typeof value === "boolean") return value;
	if (typeof value === "number") return Number.isFinite(value) ? value : undefined;
	if (typeof value === "string") return cut(value.trim(), LIMITS.cell);
	if (isObject(value) && typeof value.url === "string") {
		const text = optionalText(value.text, LIMITS.cell);
		return text === undefined ? { url: cut(value.url, LIMITS.cell) } : { text, url: cut(value.url, LIMITS.cell) };
	}
	return undefined;
}

function normalizeTable(raw: unknown): Json {
	const { fail, check } = problems("table");
	if (!isObject(raw) || !Array.isArray(raw.columns)) throw new Error("spec.columns must be an array");
	const rowsRaw = raw.rows ?? [];
	if (!Array.isArray(rowsRaw)) throw new Error("spec.rows must be an array");
	if (raw.columns.length === 0) fail("a table needs at least one column");
	if (raw.columns.length > LIMITS.columns) fail(`too many columns (${raw.columns.length}, at most ${LIMITS.columns})`);
	if (rowsRaw.length > LIMITS.rows) fail(`too many rows (${rowsRaw.length}, at most ${LIMITS.rows})`);
	const columns: Json[] = [];
	raw.columns.forEach((c, i) => {
		const key = isObject(c) ? idText(c.key ?? c.id) : idText(c);
		if (key === undefined) return fail(`column #${i + 1} has no key`);
		if (columns.some((other) => other.key === key)) return fail(`duplicate column key "${key}"`);
		const given = isObject(c) ? lower(c.type) : "";
		const type = COLUMN_TYPES.includes(given) ? given : "text";
		const align = isObject(c) && ALIGNS.includes(lower(c.align)) ? lower(c.align) : type === "number" ? "right" : "left";
		columns.push({ key, label: (isObject(c) && optionalText(c.label, MAX.label)) || key, type, align });
	});
	const rows: Json[] = [];
	rowsRaw.slice(0, LIMITS.rows).forEach((r, i) => {
		const id = (isObject(r) && idText(r.id)) || `row-${i + 1}`;
		if (rows.some((other) => other.id === id)) return fail(`duplicate row id "${id}"`);
		if (!isObject(r) || (r.cells !== undefined && !isObject(r.cells) && !Array.isArray(r.cells))) {
			return fail(`row "${id}" must be an object whose "cells" is an object or an array`);
		}
		const cells: Json = {};
		columns.forEach((column, at) => {
			const source = Array.isArray(r.cells) ? r.cells[at] : isObject(r.cells) ? r.cells[column.key as string] : r[column.key as string];
			const value = cleanCell(source);
			if (value !== undefined && value !== null) cells[column.key as string] = value;
		});
		rows.push({ id, cells, ref: normalizeRef(r.ref, `row "${id}"`, fail) });
	});
	let sort: Json | undefined;
	if (raw.sort !== undefined && raw.sort !== null) {
		const column = isObject(raw.sort) ? idText(raw.sort.column ?? raw.sort.key) : idText(raw.sort);
		if (column === undefined) fail(`"sort" needs a column`);
		else if (!columns.some((c) => c.key === column)) fail(`sort column "${column}" is not a column`);
		else sort = { column, direction: isObject(raw.sort) && lower(raw.sort.direction) === "desc" ? "desc" : "asc" };
	}
	check();
	const selectable = raw.selectable !== false;
	const wanted = new Set(list(raw.selection).flatMap((id) => idText(id) ?? []));
	const selection = selectable ? rows.filter((r) => wanted.has(r.id as string)).map((r) => r.id) : [];
	const live = normalizeLive(raw.live, fail);
	if (isObject(live?.map) && isObject(live.map.columns)) {
		for (const key of Object.keys(live.map.columns)) if (!columns.some((c) => c.key === key)) fail(`live.map.columns: "${key}" is not a column`);
	}
	check();
	return clone({ columns, rows, sort, selectable, editable: raw.editable === true, selection, live });
}

function patchTable(spec: Json, patch: Json): Json {
	const next = clone(spec) as { columns: Json[]; rows: Json[] } & Json;
	const goneRows = removed(patch, "rows");
	const goneColumns = removed(patch, "columns");
	next.rows = next.rows.filter((r) => !goneRows.has(r.id as string));
	next.columns = next.columns.filter((c) => !goneColumns.has(c.key as string));
	upsert(next.columns, patch.columns, "key", "columns");
	upsert(next.rows, patch.rows, "id", "rows", (found, item) => {
		const cells = found.cells as Json;
		const changes = isObject(item.cells) ? item.cells : Object.fromEntries(Object.entries(item).filter(([k]) => k !== "id"));
		for (const [k, v] of Object.entries(changes)) v === null ? delete cells[k] : (cells[k] = v);
	});
	scalars(next, patch, ["sort", "selection", "editable", "selectable", "live"]);
	return normalizeTable(next);
}

function summaryTable(surface: Surface): string {
	const spec = surface.spec as { columns: Json[]; rows: Json[]; sort?: Json; editable: boolean; selectable: boolean; selection: string[] };
	const flags = [spec.editable ? "editable" : "read-only", spec.selection.length > 0 ? `${spec.selection.length} selected` : ""].filter(Boolean);
	const lines = [
		`Table "${surface.title}" (${surface.id}, v${surface.version}) is shown to the user ` +
			`(${plural(spec.columns.length, "column")}, ${plural(spec.rows.length, "row")}; ${flags.join(", ")}).`,
		listed("Columns", spec.columns.map((c) => `${c.key} "${c.label}" (${c.type})`)),
	];
	for (const row of spec.rows.slice(0, LIMITS.summaryRows)) {
		const cells = row.cells as Json;
		lines.push(`${row.id}: ${spec.columns.map((c) => cut(cellText(cells[c.key as string]), 40) || "—").join(" | ")}`);
	}
	if (spec.rows.length > LIMITS.summaryRows) lines.push(`… and ${spec.rows.length - LIMITS.summaryRows} more rows`);
	if (spec.sort) lines.push(`Sorted by ${spec.sort.column} (${spec.sort.direction}).`);
	if (spec.selection.length > 0) lines.push(listed("Selected", spec.selection));
	lines.push(`The user can sort and filter${spec.selectable ? ", select rows" : ""}${spec.editable ? " and edit cells" : ""}; ` +
		`their selection and edits arrive as "${EVENT_MESSAGE}" messages.`);
	return lines.join("\n");
}

function isDate(text: string): boolean {
	const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
	if (!match) return false;
	const date = new Date(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])));
	return date.getUTCMonth() === Number(match[2]) - 1 && date.getUTCDate() === Number(match[3]);
}

/** `value` in `field`'s shape, or undefined when it doesn't fit (as `Field::coerce`). */
function fieldValue(field: Json, value: unknown): unknown {
	const type = field.type as string;
	const min = field.min as number | string | undefined;
	const max = field.max as number | string | undefined;
	const options = list(field.options).map((o) => (isObject(o) ? o.value : undefined));
	if (value === null || value === undefined || (typeof value === "string" && value.trim() === "")) {
		return type === "checkbox" ? false : type === "multiselect" ? [] : null;
	}
	switch (type) {
		case "text":
		case "textarea": {
			if (typeof value !== "string" && typeof value !== "number" && typeof value !== "boolean") return undefined;
			const text = String(value).trim();
			const length = Array.from(text).length;
			if ((typeof min === "number" && length < min) || (typeof max === "number" && length > max)) return undefined;
			return cut(text, type === "textarea" ? MAX.text * 4 : MAX.text);
		}
		case "number":
		case "slider": {
			const n = num(value);
			if (n === undefined || (typeof min === "number" && n < min) || (typeof max === "number" && n > max)) return undefined;
			return n;
		}
		case "checkbox":
			return typeof value === "boolean" ? value : undefined;
		case "select":
		case "radio":
			return typeof value === "string" && options.includes(value) ? value : undefined;
		case "multiselect": {
			if (!Array.isArray(value) || !value.every((v) => options.includes(v))) return undefined;
			const chosen = options.filter((o) => value.includes(o));
			if ((typeof min === "number" && chosen.length < min) || (typeof max === "number" && chosen.length > max)) return undefined;
			return chosen;
		}
		case "date": {
			const text = typeof value === "string" ? value.trim() : "";
			if (!isDate(text) || (typeof min === "string" && text < min) || (typeof max === "string" && text > max)) return undefined;
			return text;
		}
	}
	return undefined;
}

function normalizeForm(raw: unknown): Json {
	const { fail, check } = problems("form");
	if (!isObject(raw) || !Array.isArray(raw.fields)) throw new Error("spec.fields must be an array");
	if (raw.fields.length === 0) fail("a form needs at least one field");
	if (raw.fields.length > LIMITS.fields) fail(`too many fields (${raw.fields.length}, at most ${LIMITS.fields})`);
	const fields: Json[] = [];
	raw.fields.slice(0, LIMITS.fields).forEach((f, i) => {
		const id = isObject(f) ? idText(f.id) : undefined;
		if (id === undefined || !isObject(f)) return fail(`field #${i + 1} has no id`);
		if (fields.some((other) => other.id === id)) return fail(`duplicate field id "${id}"`);
		const type = f.type === undefined || f.type === null ? "text" : lower(f.type);
		if (!FIELD_TYPES.includes(type)) return fail(`field "${id}": unknown type ${JSON.stringify(f.type)} (one of ${FIELD_TYPES.join(", ")})`);
		const options = list(f.options).slice(0, LIMITS.options).flatMap((o) => {
			const value = isObject(o) ? idText(o.value) : idText(o);
			return value === undefined ? [] : [{ value, label: (isObject(o) && optionalText(o.label, MAX.label)) || value }];
		});
		if (["select", "multiselect", "radio"].includes(type) && options.length === 0) fail(`field "${id}" (${type}) needs options`);
		const isDateField = type === "date";
		const bound = (key: "min" | "max") => {
			if (isDateField) return typeof f[key] === "string" && isDate((f[key] as string).trim()) ? (f[key] as string).trim() : undefined;
			return num(f[key]) ?? (type === "slider" ? (key === "min" ? 0 : 100) : undefined);
		};
		const min = bound("min");
		const max = bound("max");
		if (typeof min === "number" && typeof max === "number" && min > max) fail(`field "${id}": min is greater than max`);
		const step = num(f.step);
		const field: Json = {
			id, type, label: optionalText(f.label, MAX.label) ?? id, help: optionalText(f.help, MAX.text),
			placeholder: optionalText(f.placeholder, MAX.label), required: f.required === true ? true : undefined,
			options: options.length > 0 ? options : undefined, min, max, step: step !== undefined && step > 0 ? step : undefined,
		};
		const fallback = fieldValue(field, f.default);
		const empty = fallback === undefined || fallback === null || (Array.isArray(fallback) && fallback.length === 0) || fallback === "";
		if (!empty || typeof fallback === "boolean") field.default = fallback;
		fields.push(field);
	});
	check();
	const values: Json = {};
	const given = isObject(raw.values) ? raw.values : {};
	for (const field of fields) {
		const value = fieldValue(field, given[field.id as string]);
		if (field.id as string in given && value !== undefined) values[field.id as string] = value;
	}
	return clone({
		description: optionalText(raw.description, MAX.text),
		submitLabel: optionalText(raw.submitLabel, 40) ?? "Submit",
		fields, values, submitted: raw.submitted === true,
	});
}

/** An update reopens a submitted form unless the patch says `submitted: true`. */
function patchForm(spec: Json, patch: Json): Json {
	const next = clone(spec) as { fields: Json[]; values: Json } & Json;
	const gone = removed(patch, "fields");
	next.fields = next.fields.filter((f) => !gone.has(f.id as string));
	upsert(next.fields, patch.fields, "id", "fields");
	if (isObject(patch.values)) for (const [k, v] of Object.entries(patch.values)) v === null ? delete next.values[k] : (next.values[k] = v);
	scalars(next, patch, ["description", "submitLabel"]);
	next.submitted = patch.submitted === true;
	return normalizeForm(next);
}

function shownValue(field: Json, value: unknown): string {
	const label = (v: unknown) => ((field.options as Json[] | undefined) ?? []).find((o) => o.value === v)?.label ?? String(v);
	if (value === null || value === undefined || (Array.isArray(value) && value.length === 0)) return "—";
	if (Array.isArray(value)) return value.map(label).join(", ");
	if (typeof value === "boolean") return value ? "yes" : "no";
	return cut(String(field.options ? label(value) : value), 80);
}

function summaryForm(surface: Surface): string {
	const spec = surface.spec as { fields: Json[]; values: Json; submitted: boolean; description?: string };
	const lines = [
		`Form "${surface.title}" (${surface.id}, v${surface.version}) is shown to the user ` +
			`(${plural(spec.fields.length, "field")}; ${spec.submitted ? "submitted" : "waiting for the user to submit"}).`,
		listed("Fields", spec.fields.map((f) => {
			const options = f.options ? `: ${(f.options as Json[]).map((o) => o.value).join("|")}` : "";
			return `${f.id} "${f.label}" (${f.type}${options}${f.required ? ", required" : ""})`;
		})),
	];
	const answered = spec.fields.filter((f) => (f.id as string) in spec.values);
	if (answered.length > 0) {
		lines.push(listed(spec.submitted ? "Answers" : "Prefilled", answered.map((f) => `${f.id} = ${shownValue(f, spec.values[f.id as string])}`)));
	}
	lines.push(spec.submitted
		? "The form is read-only now; update it to ask again (an update reopens it)."
		: `When the user submits, the answers arrive as a "${EVENT_MESSAGE}" message (op "submit"); wait for it.`);
	return lines.join("\n");
}

function normalizeChecklist(raw: unknown): Json {
	const { fail, check } = problems("checklist");
	if (!isObject(raw) || !Array.isArray(raw.items)) throw new Error("spec.items must be an array");
	const total = raw.items.reduce((n: number, item) => n + 1 + list(isObject(item) ? item.items : undefined).length, 0);
	if (total > LIMITS.items) fail(`too many items (${total}, at most ${LIMITS.items})`);
	const seen = new Set<string>();
	let counter = 0;
	const read = (item: unknown): Json | undefined => {
		counter += 1;
		const label = typeof item === "string" ? optionalText(item, LIMITS.itemLabel) : isObject(item) ? optionalText(item.label ?? item.text, LIMITS.itemLabel) : undefined;
		const given = isObject(item) ? idText(item.id) : undefined;
		const text = label ?? given;
		if (text === undefined) return fail(`item #${counter} has no label`);
		const id = given ?? slugOf(text, "item", (s) => seen.has(s));
		if (seen.has(id)) return fail(`duplicate item id "${id}"`);
		seen.add(id);
		// `status` wins over `done`; only running, failed and skipped are written out.
		const named = isObject(item) ? lower(item.status) : "";
		const status = RUN_STATUSES.includes(named) ? named : isObject(item) && item.done === true ? "done" : "pending";
		return {
			id, label: text, done: status === "done", status: status === "done" || status === "pending" ? undefined : status,
			note: isObject(item) ? optionalText(item.note, MAX.text) : undefined,
			ref: normalizeRef(isObject(item) ? item.ref : undefined, `item "${id}"`, fail),
		};
	};
	const items: Json[] = [];
	for (const top of raw.items.slice(0, LIMITS.items)) {
		const item = read(top);
		if (item === undefined) continue;
		const subs: Json[] = [];
		for (const sub of list(isObject(top) ? top.items : undefined)) {
			if (isObject(sub) && list(sub.items).length > 0) fail(`sub-items of "${item.id}" can't have their own items (one level only)`);
			const read_ = read(sub);
			if (read_) subs.push(read_);
		}
		if (subs.length > 0) item.items = subs;
		items.push(item);
	}
	const live = normalizeLive(raw.live, fail);
	check();
	let run = raw.run === true ? "idle" : RUN_MODES.includes(lower(raw.run)) ? lower(raw.run) : undefined;
	// A run is idle again once nothing runs and it is through (as `Checklist::settle`).
	const { next, running } = planState(items);
	if (!running && (run === "step" || (run === "all" && next === undefined))) run = "idle";
	return clone({ items, run, live });
}

/** A run plan's steps (items without sub-items), the first pending one and the running one. */
interface Plan {
	steps: Json[];
	statusOf(item: Json): string;
	next?: Json;
	running?: Json;
	/** 1-based, among the steps. */
	number(item: Json): number;
}

function planState(items: Json[]): Plan {
	const all = items.flatMap((top) => [top, ...((top.items as Json[] | undefined) ?? [])]);
	const steps = all.filter((item) => list(item.items).length === 0);
	const statusOf = (item: Json) => (item.status as string | undefined) ?? (item.done ? "done" : "pending");
	return {
		steps,
		statusOf,
		next: steps.find((item) => statusOf(item) === "pending"),
		running: all.find((item) => statusOf(item) === "running"),
		number: (item: Json) => steps.indexOf(item) + 1,
	};
}

/** Items upsert by id at either level; new ones go to the end of the top
 * level or of `parent`'s sub-items; `parent` on an existing item moves it.
 * An item's `done` or `status` replaces the other; `run` and `live` replace. */
function patchChecklist(spec: Json, patch: Json): Json {
	const next = clone(spec) as { items: Json[] } & Json;
	const gone = removed(patch, "items");
	next.items = next.items.filter((i) => !gone.has(i.id as string));
	for (const top of next.items) if (Array.isArray(top.items)) top.items = (top.items as Json[]).filter((s) => !gone.has(s.id as string));
	const lists = () => [next.items, ...next.items.map((top) => (top.items ??= []) as Json[])];
	const find = (id: string) => lists().find((l) => l.some((i) => i.id === id));
	const changes = patch.items;
	if (changes !== undefined && changes !== null) {
		if (!Array.isArray(changes) || !changes.every(isObject)) throw new Error("patch.items must be an array of objects");
		for (const item of changes) {
			const id = idText(item.id);
			if (id === undefined) throw new Error("every item in patch.items needs an id");
			const home = find(id);
			const parentGiven = "parent" in item;
			const parent = idText(item.parent);
			const target = parent === undefined ? next.items : next.items.find((t) => t.id === parent)?.items as Json[] | undefined;
			if (parentGiven && target === undefined) throw new Error(`patch.items: "${parent}" is not a top-level item`);
			let found = home?.find((i) => i.id === id);
			if (found && parentGiven && home !== target) {
				home!.splice(home!.indexOf(found), 1);
				target!.push(found);
			}
			if (!found) {
				found = { id };
				(parentGiven ? target! : next.items).push(found);
			}
			if ("done" in item && !("status" in item)) delete found.status;
			if ("status" in item && !("done" in item)) delete found.done;
			for (const [k, v] of Object.entries(item)) if (k !== "id" && k !== "parent") v === null ? delete found[k] : (found[k] = v);
		}
	}
	scalars(next, patch, ["run", "live"]);
	return normalizeChecklist(next);
}

const MARKS: Record<string, string> = { pending: " ", done: "x", running: "~", failed: "!", skipped: "-" };

function summaryChecklist(surface: Surface): string {
	const spec = surface.spec as { items: Json[]; run?: string };
	const items = spec.items;
	const plan = planState(items);
	const all = items.flatMap((top) => [top, ...((top.items as Json[] | undefined) ?? [])]);
	const lines = [`Checklist "${surface.title}" (${surface.id}, v${surface.version}) is shown to the user (${all.filter((i) => i.done).length} of ${all.length} done).`];
	const line = (item: Json, indent: string) => {
		const status = plan.statusOf(item);
		const word = status === "done" || status === "pending" ? "" : ` (${status})`;
		return `${indent}- [${MARKS[status] ?? " "}] ${item.id} ${quoted(item.label, 80)}${word}${item.note ? ` — ${cut(String(item.note), 80)}` : ""}`;
	};
	for (const top of items) {
		lines.push(line(top, ""));
		for (const sub of (top.items as Json[] | undefined) ?? []) lines.push(line(sub, "  "));
	}
	if (lines.length > LIMITS.summaryLines) lines.splice(LIMITS.summaryLines, lines.length, `… and ${lines.length - LIMITS.summaryLines} more items`);
	if (spec.run === undefined) {
		lines.push(`The user can check, add, edit, remove and reorder items; their edits arrive as "${EVENT_MESSAGE}" messages.`);
	} else {
		lines.push(runNext(spec.run, plan));
	}
	return lines.join("\n");
}

/** What a run plan expects of the agent now. */
function runNext(run: string, plan: Plan): string {
	const name = (item: Json) => `step ${plan.number(item)} ${quoted(item.label, 80)} (${item.id})`;
	const report = `report it with gentle_ui update {patch: {items: [{id, status: "done"}]}} (or "failed" with a note saying why)`;
	if (plan.running) {
		return `Run plan: ${name(plan.running)} is running: when it is through, ${report}` +
			(run === "all" ? ", then go on with the next pending step." : ", then stop and wait for the user.");
	}
	if (run === "all" && plan.next) {
		return `Run plan: the user is running it. Do ${name(plan.next)} next: first update it to status "running", do it, then ${report}; then continue with the next pending step.`;
	}
	return `Run plan: the user skips and reorders steps and presses Run or Run next; wait for their "${EVENT_MESSAGE}" saying which step to do, and report each step's status with update patches.`;
}

/** The checklist events that drive the agent: run, stop, run plan on. */
function checklistInstructions(_before: Surface, after: Surface, changes: unknown[]): string[] {
	const spec = after.spec as { items: Json[] };
	const plan = planState(spec.items);
	const lines: string[] = [];
	for (const change of changes) {
		if (!isObject(change)) continue;
		if (change.op === "run") {
			const item = plan.steps.find((s) => s.id === change.item);
			if (item === undefined) continue;
			lines.push(`Do step ${plan.number(item)} next: ${quoted(item.label, 120)} (${item.id}); it is marked running. ` +
				`Then report it with gentle_ui update {patch: {items: [{id: "${item.id}", status: "done"}]}} (or "failed" with a note)` +
				(change.mode === "all"
					? ", and go on through the next pending steps the same way (mark each running, do it, report it) until none is pending or the user stops the plan."
					: ", then stop and wait for the user."));
		} else if (change.op === "stop") {
			lines.push("The user stopped the plan: don't start another step. If one is running, finish or abandon it and report its status.");
		} else if (change.op === "run_mode" && change.on === true) {
			lines.push("The user turned this checklist into a run plan: wait for Run or Run next before doing a step.");
		}
	}
	return lines;
}

function normalizeKanban(raw: unknown): Json {
	const { fail, check } = problems("kanban");
	if (!isObject(raw) || !Array.isArray(raw.columns)) throw new Error("spec.columns must be an array");
	const cardsRaw = raw.cards ?? [];
	if (!Array.isArray(cardsRaw)) throw new Error("spec.cards must be an array");
	if (raw.columns.length === 0) fail("a board needs at least one column");
	if (raw.columns.length > LIMITS.boardColumns) fail(`too many columns (${raw.columns.length}, at most ${LIMITS.boardColumns})`);
	if (cardsRaw.length > LIMITS.cards) fail(`too many cards (${cardsRaw.length}, at most ${LIMITS.cards})`);
	const columns: Json[] = [];
	raw.columns.slice(0, LIMITS.boardColumns).forEach((c, i) => {
		const id = typeof c === "string" ? slugOf(c, "column", (s) => columns.some((o) => o.id === s)) : isObject(c) ? idText(c.id) : undefined;
		if (id === undefined) return fail(`column #${i + 1} has no id`);
		if (columns.some((o) => o.id === id)) return fail(`duplicate column id "${id}"`);
		const limit = isObject(c) && Number.isInteger(c.limit) && (c.limit as number) > 0 ? Math.min(c.limit as number, 999) : undefined;
		const label = typeof c === "string" ? optionalText(c, MAX.label) : isObject(c) ? optionalText(c.label, MAX.label) : undefined;
		columns.push({ id, label: label ?? id, limit, starts: isObject(c) && c.starts === true ? true : undefined });
	});
	const cards: Json[] = [];
	cardsRaw.slice(0, LIMITS.cards).forEach((c, i) => {
		const given = isObject(c) ? idText(c.id) : undefined;
		const title = (isObject(c) ? optionalText(c.title ?? c.label, LIMITS.cardTitle) : undefined) ?? given;
		if (title === undefined || !isObject(c)) return fail(`card #${i + 1} has no title`);
		const id = given ?? slugOf(title, "card", (s) => cards.some((o) => o.id === s));
		if (cards.some((o) => o.id === id)) return fail(`duplicate card id "${id}"`);
		const column = idText(c.column) ?? (columns[0]?.id as string | undefined);
		if (column === undefined) return;
		if (!columns.some((o) => o.id === column)) return fail(`card "${id}" is in column "${column}", which is not in columns`);
		const tags = [...new Set(list(c.tags).flatMap((t) => (typeof t === "string" && t.trim() !== "" ? [cut(t.trim(), LIMITS.tag)] : [])))].slice(0, LIMITS.tags);
		const agent: Json = typeof c.agent === "string"
			? { name: optionalText(c.agent, MAX.label) }
			: isObject(c.agent) ? { name: optionalText(c.agent.name, MAX.label), task: optionalText(c.agent.task, MAX.text) } : {};
		cards.push({
			id, title, column, description: optionalText(c.description, MAX.text),
			tags: tags.length > 0 ? tags : undefined, assignee: optionalText(c.assignee, MAX.label), ref: normalizeRef(c.ref, `card "${id}"`, fail),
			agent: agent.name === undefined && agent.task === undefined ? undefined : agent,
			agentId: idText(c.agentId),
		});
	});
	check();
	// Grouped by column, in column order (stable within a column).
	const order = (card: Json) => columns.findIndex((c) => c.id === card.column);
	cards.sort((a, b) => order(a) - order(b));
	return clone({ columns, cards });
}

function patchKanban(spec: Json, patch: Json): Json {
	const next = clone(spec) as { columns: Json[]; cards: Json[] };
	const goneColumns = removed(patch, "columns");
	const goneCards = removed(patch, "cards");
	next.columns = next.columns.filter((c) => !goneColumns.has(c.id as string));
	next.cards = next.cards.filter((c) => !goneCards.has(c.id as string) && !goneColumns.has(c.column as string));
	upsert(next.columns, patch.columns, "id", "columns");
	upsert(next.cards, patch.cards, "id", "cards");
	return normalizeKanban(next);
}

function summaryKanban(surface: Surface): string {
	const spec = surface.spec as { columns: Json[]; cards: Json[] };
	const lines = [`Board "${surface.title}" (${surface.id}, v${surface.version}) is shown to the user ` +
		`(${plural(spec.cards.length, "card")} in ${plural(spec.columns.length, "column")}).`];
	for (const column of spec.columns) {
		const cards = spec.cards.filter((c) => c.column === column.id);
		const count = column.limit ? `${cards.length}/${column.limit}` : `${cards.length}`;
		const shown = cards.map((c) =>
			`${c.id} ${quoted(c.title, 60)}${c.assignee ? ` @${c.assignee}` : ""}${c.tags ? ` [${(c.tags as string[]).join(", ")}]` : ""}` +
			`${c.agentId ? ` [subagent ${c.agentId}]` : isObject(c.agent) ? ` [for ${c.agent.name ?? "a subagent"}]` : ""}`);
		const starts = column.starts ? ", dropping a card here starts a subagent" : "";
		lines.push(listed(`${column.label} (${column.id}, ${count}${starts})`, shown).replace(/: $/, ""));
	}
	lines.push(`The user can drag cards between and within columns, add, edit and remove them; their edits arrive as "${EVENT_MESSAGE}" messages.`);
	return lines.join("\n");
}

/** A card dropped into a `starts` column: dispatch a subagent for it (the
 * app can't start one: Desktop RPC has no start method by design). */
function kanbanInstructions(_before: Surface, after: Surface, changes: unknown[]): string[] {
	const cards = (after.spec as { cards: Json[] }).cards;
	return changes.flatMap((change) => {
		if (!isObject(change) || change.op !== "dispatch") return [];
		const card = cards.find((c) => c.id === change.card);
		if (card === undefined) return [];
		const agent = isObject(card.agent) ? card.agent : {};
		const task = typeof agent.task === "string" ? agent.task : `${String(card.title)}${card.description ? `: ${String(card.description)}` : ""}`;
		return [
			`Dispatch a subagent for the card ${quoted(card.title, 80)} (${card.id})${typeof agent.name === "string" ? ` using the ${agent.name} agent` : ""} with this task: ${task}. ` +
				`Use your own subagent tool; once it is dispatched, call gentle_ui update with patch {cards: [{id: "${card.id}", agentId: "<the subagent's id>"}]} so the card follows its live status.`,
		];
	});
}

function normalizeChart(raw: unknown): Json {
	const { fail, check } = problems("chart");
	if (!isObject(raw)) throw new Error("spec must be an object with a series array");
	const type = raw.type === undefined || raw.type === null ? "bar" : lower(raw.type);
	if (!CHART_TYPES.includes(type)) throw new Error(`unknown chart type ${JSON.stringify(raw.type)} (one of ${CHART_TYPES.join(", ")})`);
	if (!Array.isArray(raw.series)) throw new Error("spec.series must be an array");
	if (raw.series.length === 0) fail("a chart needs at least one series");
	if (raw.series.length > LIMITS.series) fail(`too many series (${raw.series.length}, at most ${LIMITS.series})`);
	const labelsRaw = list(raw.labels);
	if (labelsRaw.length > LIMITS.points) fail(`too many labels (${labelsRaw.length}, at most ${LIMITS.points})`);
	let labels = labelsRaw.slice(0, LIMITS.points).map((l) => (typeof l === "string" ? cut(l.trim(), MAX.label) : typeof l === "number" ? String(l) : ""));
	const round = type === "pie" || type === "donut";
	const series: Json[] = raw.series.slice(0, LIMITS.series).map((s, i) => {
		const name = (isObject(s) && (optionalText(s.name ?? s.label, MAX.label) ?? idText(s.id))) || `Series ${i + 1}`;
		const quote = `series “${name}”`;
		if (type === "scatter") {
			const points = list(isObject(s) ? s.points ?? s.values : undefined);
			if (points.length > LIMITS.points) fail(`${quote} has too many points (${points.length}, at most ${LIMITS.points})`);
			return {
				name,
				points: points.slice(0, LIMITS.points).flatMap((p, j) => {
					const xy = Array.isArray(p) && p.length === 2 ? [num(p[0]), num(p[1])] : isObject(p) ? [num(p.x), num(p.y)] : [];
					if (xy.length === 2 && xy[0] !== undefined && xy[1] !== undefined) return [xy];
					fail(`${quote} point #${j + 1} must be [x, y] or {x, y} numbers`);
					return [];
				}),
			};
		}
		const values = list(isObject(s) ? s.values ?? s.data : undefined);
		if (values.length > LIMITS.points) fail(`${quote} has too many values (${values.length}, at most ${LIMITS.points})`);
		return {
			name,
			values: values.slice(0, LIMITS.points).flatMap((v, j) => {
				const n = num(v);
				if (n === undefined) return fail(`${quote} value #${j + 1} is not a number`) ?? [];
				if (round && n < 0) return fail(`${quote} value #${j + 1} is negative; a ${type} chart needs values ≥ 0`) ?? [];
				return [n];
			}),
		};
	});
	if (type !== "scatter") {
		if (labels.length === 0) labels = Array.from({ length: Math.max(0, ...series.map((s) => (s.values as number[]).length)) }, (_, i) => String(i + 1));
		for (const s of series) {
			const count = (s.values as number[]).length;
			if (count !== labels.length) fail(`series “${s.name}” has ${plural(count, "value")} but there are ${plural(labels.length, "label")}`);
		}
	}
	const live = normalizeLive(raw.live, fail);
	if (live !== undefined && type === "scatter") fail("a live chart needs labels: any type but scatter");
	check();
	return clone({
		type, labels: type === "scatter" ? undefined : labels, series,
		xTitle: optionalText(raw.xTitle, MAX.label), yTitle: optionalText(raw.yTitle, MAX.label), unit: optionalText(raw.unit, 16),
		stacked: raw.stacked === true && (type === "bar" || type === "area") ? true : undefined,
		live: type === "scatter" ? undefined : live,
	});
}

/** Series upsert by name; top-level keys replace. */
function patchChart(spec: Json, patch: Json): Json {
	const next = clone(spec) as { series: Json[] } & Json;
	const gone = removed(patch, "series");
	next.series = next.series.filter((s) => !gone.has(s.name as string));
	upsert(next.series, patch.series, "name", "series");
	scalars(next, patch, ["type", "labels", "xTitle", "yTitle", "unit", "stacked", "live"]);
	return normalizeChart(next);
}

function summaryChart(surface: Surface): string {
	const spec = surface.spec as { type: string; labels?: string[]; series: Json[]; xTitle?: string; yTitle?: string; unit?: string };
	const count = spec.labels ? ` over ${plural(spec.labels.length, "label")}` : "";
	const lines = [`Chart "${surface.title}" (${surface.id}, v${surface.version}) is shown to the user: ` +
		`${spec.type} chart, ${spec.series.length} series${count}` +
		`${spec.unit ? ` in ${spec.unit}` : ""}${spec.yTitle ? `, y: ${spec.yTitle}` : ""}${spec.xTitle ? `, x: ${spec.xTitle}` : ""}.`];
	if (spec.labels) lines.push(listed("Labels", spec.labels));
	for (const s of spec.series) {
		const values = s.points ? (s.points as number[][]).map(([x, y]) => `(${x}, ${y})`) : (s.values as number[]).map(String);
		lines.push(listed(String(s.name), values));
	}
	lines.push("It is read-only; the user can hover values and ask about one in their next message.");
	return lines.join("\n");
}

// ── review: a change set reviewed hunk by hunk before the agent writes it ───
// Mirrors `gentle_core::genui::review`. The extension alone turns a file's
// `before`/`after` into hunks (`lineDiff`): the app only ever sees hunks.

const REVIEW = { files: 50, hunks: 300, hunkLines: 1000, lines: 6000, line: 1000, path: 300, header: 120, context: 3, edits: 2000 };
const FILE_STATUS: Record<string, string> = {
	added: "added", add: "added", new: "added", created: "added",
	modified: "modified", modify: "modified", changed: "modified", edit: "modified", edited: "modified",
	deleted: "deleted", delete: "deleted", removed: "deleted",
	renamed: "renamed", rename: "renamed", moved: "renamed",
};
const LINE_KINDS: Record<string, string> = {
	context: "context", ctx: "context", "": "context", same: "context", unchanged: "context",
	add: "add", added: "add", "+": "add", insert: "add", ins: "add",
	del: "del", delete: "del", deleted: "del", remove: "del", removed: "del", "-": "del",
};
const DECISIONS: Record<string, string> = { accepted: "accepted", accept: "accepted", rejected: "rejected", reject: "rejected" };
// A `wait: true` call gives up after this long (the review stays open).
const REVIEW_WAIT_MS = 30 * 60 * 1000;

interface DiffLine { kind: string; text: string }

/** `text` as lines: one trailing newline ends the last line, `\r` dropped. */
function textLines(text: string): string[] {
	if (text === "") return [];
	return text.replace(/\n$/, "").split("\n").map((line) => line.replace(/\r$/, ""));
}

/** The lines of `after` against `before` (Myers' O(ND) diff on lines; a
 * plain replacement when they differ in more than REVIEW.edits places). */
function lineDiff(before: string[], after: string[]): DiffLine[] {
	let start = 0;
	while (start < before.length && start < after.length && before[start] === after[start]) start += 1;
	let endA = before.length;
	let endB = after.length;
	while (endA > start && endB > start && before[endA - 1] === after[endB - 1]) {
		endA -= 1;
		endB -= 1;
	}
	const a = before.slice(start, endA);
	const b = after.slice(start, endB);
	const n = a.length;
	const m = b.length;
	const middle: DiffLine[] = [];
	const offset = n + m + 1;
	const v = new Int32Array(2 * offset + 1);
	// trace[d]: v for k in -d..d before step d.
	const trace: Int32Array[] = [];
	let found = n + m === 0;
	for (let d = 0; d <= Math.min(n + m, REVIEW.edits) && !found; d += 1) {
		trace.push(v.slice(offset - d, offset + d + 1));
		for (let k = -d; k <= d; k += 2) {
			let x = k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1]) ? v[offset + k + 1] : v[offset + k - 1] + 1;
			let y = x - k;
			while (x < n && y < m && a[x] === b[y]) {
				x += 1;
				y += 1;
			}
			v[offset + k] = x;
			if (x >= n && y >= m) {
				found = true;
				break;
			}
		}
	}
	if (!found) {
		middle.push(...a.map((text) => ({ kind: "del", text })), ...b.map((text) => ({ kind: "add", text })));
	} else {
		let x = n;
		let y = m;
		for (let d = trace.length - 1; d > 0; d -= 1) {
			const prev = trace[d];
			const at = (k: number) => prev[k + d];
			const k = x - y;
			const down = k === -d || (k !== d && at(k - 1) < at(k + 1));
			const prevK = down ? k + 1 : k - 1;
			const prevX = at(prevK);
			const prevY = prevX - prevK;
			while (x > prevX && y > prevY) {
				middle.push({ kind: "context", text: a[x - 1] });
				x -= 1;
				y -= 1;
			}
			if (down) {
				middle.push({ kind: "add", text: b[y - 1] });
				y -= 1;
			} else {
				middle.push({ kind: "del", text: a[x - 1] });
				x -= 1;
			}
		}
		while (x > 0 && y > 0) {
			middle.push({ kind: "context", text: a[x - 1] });
			x -= 1;
			y -= 1;
		}
		middle.reverse();
	}
	return [
		...before.slice(0, start).map((text) => ({ kind: "context", text })),
		...middle,
		...before.slice(endA).map((text) => ({ kind: "context", text })),
	];
}

/** Unified hunks of a line diff, with REVIEW.context lines around changes. */
function diffHunks(lines: DiffLine[]): Json[] {
	let old = 0;
	let now = 0;
	const at = lines.map((line) => {
		const place = { old, now };
		if (line.kind !== "add") old += 1;
		if (line.kind !== "del") now += 1;
		return place;
	});
	const changed = lines.flatMap((line, i) => (line.kind === "context" ? [] : [i]));
	const hunks: Json[] = [];
	let i = 0;
	while (i < changed.length) {
		let j = i;
		while (j + 1 < changed.length && changed[j + 1] - changed[j] <= 2 * REVIEW.context + 1) j += 1;
		const from = Math.max(0, changed[i] - REVIEW.context);
		const to = Math.min(lines.length, changed[j] + REVIEW.context + 1);
		const body = lines.slice(from, to);
		const oldCount = body.filter((l) => l.kind !== "add").length;
		const newCount = body.filter((l) => l.kind !== "del").length;
		hunks.push({
			oldStart: at[from].old + (oldCount > 0 ? 1 : 0),
			newStart: at[from].now + (newCount > 0 ? 1 : 0),
			lines: body,
		});
		i = j + 1;
	}
	return hunks;
}

function reviewPath(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	const path = value.trim().replace(/\\/g, "/");
	const clean = path.startsWith("./") ? path.slice(2) : path;
	return clean === "" ? undefined : clean;
}

function reviewLines(raw: unknown[], hunk: string, fail: (message: string) => undefined): DiffLine[] {
	const lines: DiffLine[] = [];
	raw.forEach((item, i) => {
		let kind: string | undefined;
		let text: string;
		if (typeof item === "string") {
			const first = item.charAt(0);
			kind = first === "+" ? "add" : first === "-" ? "del" : "context";
			text = first === "+" || first === "-" || first === " " ? item.slice(1) : item;
		} else if (isObject(item)) {
			const given = item.kind ?? item.type;
			kind = typeof given === "string" ? LINE_KINDS[given.trim().toLowerCase()] : undefined;
			if (kind === undefined) return fail(`line ${i + 1} of hunk "${hunk}" needs kind context, add or del`);
			text = typeof item.text === "string" ? item.text : "";
		} else {
			return fail(`line ${i + 1} of hunk "${hunk}" must be a string or {kind, text}`);
		}
		const body = text.endsWith("\n") ? text.slice(0, -1) : text;
		for (const part of body.split("\n")) lines.push({ kind, text: cut(part.replace(/\r$/, ""), REVIEW.line) });
	});
	return lines;
}

/** A file given as `before`/`after` text (no hunks) with its hunks computed. */
function reviewFromText(file: unknown): unknown {
	if (!isObject(file) || file.hunks !== undefined) return file;
	const before = typeof file.before === "string" ? file.before : undefined;
	const after = typeof file.after === "string" ? file.after : undefined;
	if (before === undefined && after === undefined) return file;
	const { before: _b, after: _a, ...rest } = file;
	const status = rest.status ?? (before === undefined ? "added" : after === undefined ? "deleted" : undefined);
	const lines = lineDiff(textLines(before ?? ""), textLines(after ?? ""));
	return { ...rest, ...(status === undefined ? {} : { status }), hunks: diffHunks(lines) };
}

function normalizeReview(raw: unknown): Json {
	const { fail, check } = problems("review");
	if (!isObject(raw) || !Array.isArray(raw.files)) throw new Error("spec.files must be an array");
	const rawFiles = raw.files.map(reviewFromText);
	if (rawFiles.length === 0) fail("a review needs at least one file");
	if (rawFiles.length > REVIEW.files) fail(`too many files (${rawFiles.length}, at most ${REVIEW.files})`);
	const rawHunks = rawFiles.flatMap((f) => list(isObject(f) ? f.hunks : undefined));
	if (rawHunks.length > REVIEW.hunks) fail(`too many hunks (${rawHunks.length}, at most ${REVIEW.hunks})`);
	// Given ids first, so a generated one never takes an id a later hunk names.
	const given = new Set(rawHunks.flatMap((h) => idText(isObject(h) ? h.id : undefined) ?? []));
	const seen = new Set<string>();
	let counter = 0;
	let totalLines = 0;
	const files: Json[] = [];
	rawFiles.slice(0, REVIEW.files).forEach((rawFile, fi) => {
		const item = isObject(rawFile) ? rawFile : {};
		const path = reviewPath(item.path);
		if (path === undefined) return fail(`file ${fi + 1} has no path`);
		if (Array.from(path).length > REVIEW.path) return fail(`the path of file ${fi + 1} is longer than ${REVIEW.path} characters`);
		if (files.some((f) => f.path === path)) return fail(`duplicate file "${path}"`);
		const given_ = reviewPath(item.oldPath);
		const oldPath = given_ !== path ? given_ : undefined;
		const status = typeof item.status === "string"
			? FILE_STATUS[item.status.trim().toLowerCase()] ?? "modified"
			: oldPath !== undefined ? "renamed" : "modified";
		if (status === "renamed" && oldPath === undefined) fail(`renamed file "${path}" needs oldPath`);
		if (item.hunks !== undefined && !Array.isArray(item.hunks)) fail(`the hunks of "${path}" must be an array`);
		const language = optionalText(item.language, 40)?.toLowerCase();
		const hunks: Json[] = [];
		for (const rawHunk of list(item.hunks)) {
			counter += 1;
			const h = isObject(rawHunk) ? rawHunk : {};
			let id = idText(h.id);
			if (id === undefined) {
				let n = counter;
				while (given.has(`h${n}`) || seen.has(`h${n}`)) n += 1;
				id = `h${n}`;
			}
			if (seen.has(id)) {
				fail(`duplicate hunk id "${id}"`);
				continue;
			}
			seen.add(id);
			const lines = reviewLines(list(h.lines), id, fail);
			totalLines += lines.length;
			if (lines.length > REVIEW.hunkLines) fail(`hunk "${id}" has too many lines (${lines.length}, at most ${REVIEW.hunkLines})`);
			if (!lines.some((l) => l.kind !== "context")) fail(`hunk "${id}" changes nothing`);
			const wrong = status === "added" ? lines.some((l) => l.kind !== "add") : status === "deleted" ? lines.some((l) => l.kind !== "del") : false;
			if (wrong) fail(`hunk "${id}" of ${status} file "${path}" can only ${status === "added" ? "add" : "remove"} lines`);
			const lineNumber = (value: unknown) => (typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : undefined);
			const oldStart = lineNumber(h.oldStart) ?? (status === "added" ? 0 : status === "deleted" ? 1 : undefined);
			const newStart = lineNumber(h.newStart) ?? (status === "added" ? 1 : status === "deleted" ? 0 : undefined);
			if (oldStart === undefined || newStart === undefined) {
				fail(`hunk "${id}" needs oldStart and newStart (1-based line numbers)`);
				continue;
			}
			const decision = typeof h.decision === "string" ? DECISIONS[h.decision.trim().toLowerCase()] : undefined;
			const hunk: Json = {
				id,
				header: optionalText(h.header, REVIEW.header),
				oldStart,
				newStart,
				lines,
				note: optionalText(h.note, MAX.text),
				decision,
				comment: optionalText(h.comment, MAX.text),
				applied: h.applied === true ? true : undefined,
			};
			const oldCount = lines.filter((l) => l.kind !== "add").length;
			const newCount = lines.filter((l) => l.kind !== "del").length;
			if ((oldCount > 0 && oldStart === 0) || (newCount > 0 && newStart === 0)) fail(`hunk "${id}" starts at line 0`);
			hunks.push(hunk);
		}
		// In file order; overlapping hunks can't both apply.
		const count = (h: Json, side: "old" | "new") => (h.lines as DiffLine[]).filter((l) => l.kind !== (side === "old" ? "add" : "del")).length;
		hunks.sort((x, y) => (x.oldStart as number) - (y.oldStart as number) || (x.newStart as number) - (y.newStart as number));
		for (let i = 1; i < hunks.length; i += 1) {
			const [x, y] = [hunks[i - 1], hunks[i]];
			if ((y.oldStart as number) < (x.oldStart as number) + count(x, "old") || (y.newStart as number) < (x.newStart as number) + count(x, "new")) {
				fail(`hunks "${x.id}" and "${y.id}" overlap`);
			}
		}
		if (list(item.hunks).length === 0 && status !== "renamed") fail(`file "${path}" has no hunks`);
		files.push({ path, status, oldPath: status === "renamed" ? oldPath : undefined, language, hunks });
	});
	if (totalLines > REVIEW.lines) fail(`too many lines (${totalLines}, at most ${REVIEW.lines})`);
	check();
	return clone({ summary: optionalText(raw.summary, MAX.text), files });
}

/** `applied` marks hunks written; `hunks` merge by id (null clears, e.g.
 * `decision: null` asks again); `files` upsert by path; `remove: {files, hunks}`. */
function patchReview(spec: Json, patch: Json): Json {
	const next = clone(spec) as { files: Json[] } & Json;
	const goneFiles = removed(patch, "files");
	const goneHunks = removed(patch, "hunks");
	next.files = next.files.filter((f) => !goneFiles.has(f.path as string));
	for (const file of next.files) file.hunks = (file.hunks as Json[]).filter((h) => !goneHunks.has(h.id as string));
	const hunk = (id: string, where: string): Json => {
		const found = next.files.flatMap((f) => f.hunks as Json[]).find((h) => h.id === id);
		if (!found) throw new Error(`${where}: unknown hunk "${id}" (add hunks through files)`);
		return found;
	};
	if (patch.applied !== undefined) {
		if (!Array.isArray(patch.applied)) throw new Error("patch.applied must be an array of hunk ids");
		for (const id of patch.applied) hunk(idText(id) ?? "", "patch.applied").applied = true;
	}
	if (patch.hunks !== undefined && patch.hunks !== null) {
		if (!Array.isArray(patch.hunks) || !patch.hunks.every(isObject)) throw new Error("patch.hunks must be an array of objects");
		for (const item of patch.hunks) {
			const found = hunk(idText(item.id) ?? "", "patch.hunks");
			for (const [k, v] of Object.entries(item)) if (k !== "id") v === null ? delete found[k] : (found[k] = v);
		}
	}
	if (patch.files !== undefined && patch.files !== null) {
		if (!Array.isArray(patch.files) || !patch.files.every(isObject)) throw new Error("patch.files must be an array of objects");
		for (const item of patch.files) {
			const path = reviewPath(item.path);
			if (path === undefined) throw new Error("every item in patch.files needs a path");
			const at = next.files.findIndex((f) => f.path === path);
			if (at >= 0) next.files[at] = { ...item, path };
			else next.files.push({ ...item, path });
		}
	}
	scalars(next, patch, ["summary"]);
	return normalizeReview(next);
}

const hunkRef = (file: Json, hunk: Json): string => {
	const lines = hunk.lines as DiffLine[];
	const oldCount = lines.filter((l) => l.kind !== "add").length;
	const newCount = lines.filter((l) => l.kind !== "del").length;
	return `${hunk.id}: ${file.path} @@ -${hunk.oldStart},${oldCount} +${hunk.newStart},${newCount} @@${hunk.header ? ` ${hunk.header}` : ""}`;
};

function summaryReview(surface: Surface): string {
	const files = (surface.spec as { files: Json[] }).files;
	const hunks = files.flatMap((f) => f.hunks as Json[]);
	const all = (kind: string) => hunks.reduce((n, h) => n + (h.lines as DiffLine[]).filter((l) => l.kind === kind).length, 0);
	const decided = hunks.filter((h) => h.decision !== undefined).length;
	const lines = [
		`Review "${surface.title}" (${surface.id}, v${surface.version}) is shown to the user: ${plural(files.length, "file")}, ` +
			`+${all("add")} −${all("del")}, ${decided} of ${plural(hunks.length, "hunk")} decided.`,
	];
	const spec = surface.spec as { summary?: string };
	if (spec.summary) lines.push(cut(spec.summary, 200));
	for (const file of files) {
		lines.push(`- ${file.path} (${file.status}${file.oldPath ? ` from ${file.oldPath}` : ""})`);
		for (const hunk of file.hunks as Json[]) {
			const state = hunk.applied ? "applied" : (hunk.decision as string | undefined) ?? "undecided";
			const comment = hunk.comment ? ` — comment: ${quoted(hunk.comment, 120)}` : "";
			lines.push(`  - [${state}] ${hunkRef(file, hunk)}${comment}`);
		}
	}
	if (lines.length > LIMITS.summaryLines) lines.splice(LIMITS.summaryLines, lines.length, `… and ${lines.length - LIMITS.summaryLines} more hunks`);
	const unapplied = hunks.filter((h) => h.decision === "accepted" && !h.applied).length;
	if (decided < hunks.length) {
		lines.push(`The user decides each hunk (accept, reject, comment); decisions arrive as a "${EVENT_MESSAGE}" message (op decide). Write no undecided or rejected hunk; apply only accepted ones.`);
	} else if (unapplied > 0) {
		lines.push("Every hunk is decided: apply only the accepted ones, then mark them with update patch {applied: [hunk ids]}.");
	} else {
		lines.push("Every hunk is decided and every accepted one is applied.");
	}
	return lines.join("\n");
}

/** What the agent is told about a review's newly sent decisions. */
function reviewDecisions(surface: Surface, changes: unknown[], note: string | undefined): string {
	const files = (surface.spec as { files: Json[] }).files;
	const placed = files.flatMap((file) => (file.hunks as Json[]).map((hunk) => ({ file, hunk })));
	const ref = (id: string) => {
		const found = placed.find((p) => p.hunk.id === id);
		return found ? hunkRef(found.file, found.hunk) : id;
	};
	const decided = changes.flatMap((c) => (isObject(c) && c.op === "decide" && typeof c.hunk === "string" ? [c] : []));
	const pick = (decision: string | null) => decided.filter((c) => (c.decision ?? null) === decision);
	const accepted = pick("accepted");
	const rejected = pick("rejected");
	const commented = decided.filter((c) => typeof c.comment === "string" && c.comment !== "");
	const undecided = placed.filter((p) => p.hunk.decision === undefined && !p.hunk.applied).map((p) => p.hunk.id as string);
	const lines = [`The user reviewed the changes in "${surface.title}" (${surface.id}, now v${surface.version}):`];
	if (accepted.length > 0) {
		lines.push("Accepted: apply exactly these hunks as proposed:", ...accepted.map((c) => `- ${ref(c.hunk as string)}`));
	}
	if (rejected.length > 0) {
		lines.push("Rejected: do NOT apply these hunks:", ...rejected.map((c) => `- ${ref(c.hunk as string)}`));
	}
	if (commented.length > 0) {
		lines.push(
			"Comments to address (on an accepted hunk, apply it as the comment asks; on a rejected one, it says why or what to do instead; on an undecided one, answer it or revise the hunk with update):",
			...commented.map((c) => `- ${c.hunk} (${(c.decision as string | null) ?? "undecided"}): ${quoted(c.comment, 400)}`),
		);
	}
	if (undecided.length > 0) lines.push(listed("Still undecided (don't apply these yet; decisions on them arrive later)", undecided));
	if (note) lines.push(`Note from the user: ${note}`);
	if (accepted.length > 0) {
		lines.push(`After applying, call gentle_ui update with ${JSON.stringify({ action: "update", id: surface.id, patch: { applied: accepted.map((c) => c.hunk) } })} so the user sees them applied.`);
	}
	lines.push(`Changes: ${JSON.stringify(changes)}`);
	return lines.join("\n");
}

// ── html: an agent-written page, run as an MCP App ─────────────────────────

const HTML_MAX = 512 * 1024;
const CSP_KEYS = ["connectDomains", "resourceDomains", "frameDomains", "baseUriDomains"];
const MAX_ORIGINS = 20;
// Web origins only (as the app checks): https/wss anywhere, http/ws on loopback.
const ORIGIN = /^(?:(?:https|wss):\/\/(?:\*\.)?[a-z0-9-]+(?:\.[a-z0-9-]+)*|(?:http|ws):\/\/(?:localhost|127\.0\.0\.1))(?::\d{1,5})?(?:\/[A-Za-z0-9\-._~%/]*)?$/i;

/** An html spec `{ html, height?, csp?, data? }`, checked; throws readable errors. */
function normalizeHtml(raw: unknown): Json {
	if (!isObject(raw)) throw new Error("spec must be an object: { html, height?, csp?, data? }");
	const errors: string[] = [];
	const spec: Json = {};
	if (typeof raw.html !== "string" || raw.html.trim() === "") errors.push("spec.html must be a non-empty string with the page");
	else if (new TextEncoder().encode(raw.html).length > HTML_MAX) errors.push(`spec.html is larger than ${HTML_MAX / 1024} KiB`);
	else spec.html = raw.html;
	if (raw.height !== undefined && raw.height !== null) {
		if (typeof raw.height === "number" && Number.isFinite(raw.height)) spec.height = Math.round(Math.min(2000, Math.max(120, raw.height)));
		else errors.push("spec.height must be a number (px)");
	}
	if (raw.csp !== undefined && raw.csp !== null) {
		if (!isObject(raw.csp)) errors.push("spec.csp must be an object");
		else {
			const csp: Json = {};
			for (const key of CSP_KEYS) {
				const origins = raw.csp[key];
				if (origins === undefined || origins === null) continue;
				if (!Array.isArray(origins) || origins.length > MAX_ORIGINS) {
					errors.push(`spec.csp.${key} must be an array of at most ${MAX_ORIGINS} origins`);
					continue;
				}
				const trimmed = origins.map((o) => (typeof o === "string" ? o.trim().replace(/\/+$/, "") : ""));
				const bad = trimmed.filter((o) => !ORIGIN.test(o) || /^(?:https|wss):\/\/\*\.[^.]+$/i.test(o));
				if (bad.length > 0) errors.push(`spec.csp.${key}: not origins like https://api.example.com: ${bad.map((o) => JSON.stringify(o)).join(", ")}`);
				else if (trimmed.length > 0) csp[key] = [...new Set(trimmed)];
			}
			if (Object.keys(csp).length > 0) spec.csp = csp;
		}
	}
	if (raw.data !== undefined && raw.data !== null) {
		if (isObject(raw.data)) spec.data = raw.data;
		else errors.push("spec.data must be an object");
	}
	if (errors.length > 0) throw new Error(`invalid html spec:\n- ${errors.join("\n- ")}`);
	return clone(spec);
}

/** html patch: html/height/csp replace (null clears); data merges one level (null deletes a key). */
function patchHtml(spec: Json, patch: Json): Json {
	const next: Json = { ...spec };
	for (const key of ["html", "height", "csp"]) {
		if (!(key in patch)) continue;
		if (patch[key] === null) delete next[key];
		else next[key] = patch[key];
	}
	if (patch.data === null) delete next.data;
	else if (patch.data !== undefined) {
		if (!isObject(patch.data)) throw new Error("patch.data must be an object");
		const data: Json = { ...(isObject(next.data) ? next.data : {}) };
		merge(data, patch.data);
		next.data = data;
	}
	return normalizeHtml(next);
}

function summaryHtml(surface: Surface): string {
	const { spec } = surface;
	const size = Math.ceil(new TextEncoder().encode(String(spec.html ?? "")).length / 1024);
	const csp = isObject(spec.csp) ? spec.csp : {};
	const origins = CSP_KEYS.flatMap((key) => (Array.isArray(csp[key]) ? (csp[key] as string[]) : []));
	const data = isObject(spec.data) ? Object.keys(spec.data) : [];
	return [
		`HTML UI "${surface.title}" (${surface.id}, v${surface.version}) is shown to the user as an isolated MCP App page ` +
			`(${size} KiB, ${origins.length > 0 ? `may reach ${[...new Set(origins)].join(", ")}` : "no network"}).`,
		data.length > 0 ? listed("Data keys (the page's tool input and structuredContent)", data) : "No data: the page gets an empty tool input.",
		`The user opens it from the conversation. Messages it sends arrive as "${EVENT_MESSAGE}" messages; context it shares arrives with the next turn.`,
	].join("\n");
}

function contextText(change: Json): string {
	const blocks = Array.isArray(change.content) ? change.content : [];
	const parts = blocks.flatMap((b) => (isObject(b) ? [b.type === "text" && typeof b.text === "string" ? b.text : `[${String(b.type)} omitted]`] : []));
	if (isObject(change.structuredContent)) parts.push(JSON.stringify(change.structuredContent));
	return parts.join("\n");
}

const EXAMPLE = JSON.stringify({
	action: "show",
	id: "checkout",
	title: "Checkout architecture",
	spec: {
		groups: [{ id: "backend", label: "Backend" }],
		nodes: [
			{ id: "web", label: "Web app", kind: "client" },
			{ id: "api", label: "API", group: "backend", tech: "Rust" },
			{ id: "db", label: "Orders DB", kind: "database", group: "backend" },
		],
		edges: [{ from: "web", to: "api", label: "HTTPS" }, { from: "api", to: "db", style: "data" }],
	},
});

/** What each kind brings: its default title, spec normalizing (the readable
 * hard errors), patch merging and the summary the model reads. */
interface KindDef {
	noun: string;
	title: string;
	normalize(raw: unknown): Json;
	/** `spec` with `patch` merged, validated. */
	patch(spec: Json, patch: Json): Json;
	summary(surface: Surface): string;
	/** What an event asks of the agent beyond the edits (run a step,
	 * dispatch a subagent), from the surface before and after it. */
	instructions?(before: Surface, after: Surface, changes: unknown[]): string[];
}

const KINDS: Record<string, KindDef> = {
	diagram: {
		noun: "diagram",
		title: "Diagram",
		normalize: (raw) => normalize(raw) as unknown as Json,
		patch: (spec, patch) => applyPatch(spec as unknown as Spec, patch) as unknown as Json,
		summary,
	},
	table: { noun: "table", title: "Table", normalize: normalizeTable, patch: patchTable, summary: (s) => withLive(summaryTable(s), s) },
	form: { noun: "form", title: "Form", normalize: normalizeForm, patch: patchForm, summary: summaryForm },
	checklist: {
		noun: "checklist", title: "Checklist", normalize: normalizeChecklist, patch: patchChecklist,
		summary: (s) => withLive(summaryChecklist(s), s), instructions: checklistInstructions,
	},
	kanban: { noun: "board", title: "Board", normalize: normalizeKanban, patch: patchKanban, summary: summaryKanban, instructions: kanbanInstructions },
	chart: { noun: "chart", title: "Chart", normalize: normalizeChart, patch: patchChart, summary: (s) => withLive(summaryChart(s), s) },
	html: { noun: "HTML UI", title: "HTML", normalize: normalizeHtml, patch: patchHtml, summary: summaryHtml },
	review: { noun: "review", title: "Review", normalize: normalizeReview, patch: patchReview, summary: summaryReview },
};

const KIND_EXAMPLES = [
	{ kind: "table", title: "Flaky tests", spec: { columns: [{ key: "name", label: "Test" }, { key: "fails", label: "Fails", type: "number" }, { key: "state", type: "status" }], rows: [{ id: "login", cells: { name: "login flow", fails: 4, state: "failing" } }] } },
	{ kind: "form", title: "Deploy options", spec: { fields: [{ id: "env", type: "radio", label: "Environment", options: ["staging", "prod"], required: true }, { id: "notes", type: "textarea", label: "Notes" }] } },
	{ kind: "checklist", title: "Migration plan", spec: { items: [{ id: "backup", label: "Back up the DB" }, { id: "migrate", label: "Run migrations", items: [{ id: "users", label: "users table" }] }] } },
	{ kind: "kanban", title: "Sprint", spec: { columns: [{ id: "todo", label: "To do" }, { id: "doing", label: "Doing", limit: 3 }, { id: "done", label: "Done" }], cards: [{ id: "oauth", title: "OAuth login", column: "doing", tags: ["backend"] }] } },
	{ kind: "chart", title: "p95 latency", spec: { type: "line", labels: ["Mon", "Tue", "Wed"], series: [{ name: "api", values: [120, 135, 98] }], unit: "ms" } },
	{ kind: "review", title: "Rename login", spec: { summary: "login → sign_in", files: [{ path: "src/auth.ts", hunks: [{ id: "auth-1", header: "export function", oldStart: 12, newStart: 12, lines: [" export function", "-login(user) {", "+signIn(user) {", "   return check(user);"] }] }, { path: "src/app.ts", before: "login(u);\n", after: "signIn(u);\n" }] } },
].map((example) => `gentle_ui ${example.kind} example: ${JSON.stringify({ action: "show", ...example })}`);

const Params = Type.Object({
	action: StringEnum(["show", "update", "close"] as const),
	id: Type.Optional(Type.String({ description: "Stable slug for the UI (show) or the id to update/close" })),
	title: Type.Optional(Type.String({ description: "Short title, default the kind's name (\"Diagram\", \"Table\"…)" })),
	kind: Type.Optional(StringEnum(["diagram", "table", "form", "checklist", "kanban", "chart", "html", "review"] as const, { description: "For show, default diagram; an id keeps its kind" })),
	spec: Type.Optional(Type.Object({}, {
		additionalProperties: true,
		description:
			"Full spec for the kind. diagram: { direction?: LR|TB, groups?: [{id,label,description?}], nodes: [{id,label,kind?,group?,description?,tech?,status?,ref?}], edges?: [{id?,from,to,label?,style?}] }; " +
			`node kind: ${NODE_KINDS.join("|")}, status: ${STATUSES.join("|")}, edge style: ${EDGE_STYLES.join("|")}. ` +
			`table: { columns: [{key,label?,type?: ${COLUMN_TYPES.join("|")},align?}], rows: [{id,cells:{key:value},ref?}], sort?: {column,direction:asc|desc}, editable?, selectable?, selection?: [row ids], live? }. ` +
			`form: { fields: [{id,type: ${FIELD_TYPES.join("|")},label,help?,placeholder?,required?,options?: [value|{value,label}],min?,max?,step?,default?}], description?, submitLabel?, values? }. ` +
			"checklist: { items: [{id,label,done?,status?: pending|running|done|failed|skipped,note?,ref?,items?: [sub-items, one level]}], run?: true (a plan the user runs), live? }. " +
			"kanban: { columns: [{id,label,limit?,starts?: true (a card dropped here asks you to dispatch a subagent)}], cards: [{id,title,column,description?,tags?,assignee?,ref?,agent?: {name?,task?},agentId?}] }. " +
			"ref (any item that is code): { path: repo-relative, line?, endLine? } or \"path:line\". " +
			`chart: { type: ${CHART_TYPES.join("|")}, labels: [...], series: [{name,values:[numbers, one per label]}] (scatter: points: [[x,y]]), xTitle?, yTitle?, unit?, stacked?, live? }. ` +
			`live (table, chart, checklist; the app fills the data): { source: ${LIVE_SOURCES.filter((s) => s !== "command").join("|")}, map? } or { command: {cmd, cwd?, parse: json|lines|csv}, interval?: seconds ≥ ${LIVE.minInterval}, map? }; ` +
			"map: { items?: path to the records, id?, columns?: {key: path} (table), label?, status?, note? (checklist), label?, series?: {name: path} (chart) } with JSONPath-lite paths ($.a.b, $.list[0], $.list[*], relative to each record). " +
			"html: { html: self-contained page ≤512 KiB, height?: px, data?: object (the page's tool input/result), csp?: { connectDomains?, resourceDomains?, frameDomains?, baseUriDomains? } (origins; default no network) }. " +
			"review: { summary?, files: [{path, status?: added|modified|deleted|renamed, oldPath?, language?, hunks: [{id, header?, oldStart, newStart, lines: [\"+added\"|\"-removed\"|\" context\" or {kind: context|add|del, text}], note?}]}] } or per file {path, before?, after?} (whole texts; hunks are computed; no before = added, no after = deleted).",
	})),
	patch: Type.Optional(Type.Object({}, {
		additionalProperties: true,
		description:
			"Changes for update; lists upsert (null clears a field) and remove: {...} deletes. diagram: { nodes?, edges? (by id or from+to), groups?, remove?: {nodes,edges,groups}, direction? }. " +
			"table: { rows? (by id, cells merge), columns? (by key), remove?: {rows,columns}, sort?, selection?, editable? }. " +
			"form: { fields? (by id), values?: {id: value}, remove?: {fields}, submitted? } (an update reopens a submitted form). " +
			"checklist: { items? (by id at any level; parent: id|null places it; status: pending|running|done|failed|skipped), remove?: {items}, run?, live? }. " +
			"kanban: { cards? (by id; agentId: the subagent you dispatched), columns? (by id), remove?: {cards,columns} }. chart: { series? (by name), remove?: {series}, labels?, type?, titles?, live? }. table: also live?. " +
			"html: { html?, height?, csp? (replace; null clears), data?: {merged; null deletes a key} } (data-only updates reach the open page without reloading). " +
			"review: { applied?: [hunk ids you wrote], hunks? (by id; decision: null asks again), files? (by path, replace), remove?: {files,hunks}, summary? }.",
	})),
	wait: Type.Optional(Type.Boolean({ description: "review only: wait for the user's decisions and get them as this call's result (default false: they arrive later as a message)" })),
});

export default function gentleUi(pi: ExtensionAPI) {
	const surfaces = new Map<string, Surface>();
	// The latest context each html page shared, sent when the next turn starts.
	const pendingContext = new Map<string, { surface: Surface; change: Json }>();
	// Every id seen on the branch, closed ones included, so `ui-<n>` never repeats.
	const seen = new Set<string>();
	// Reviews a `wait: true` call is waiting on, by id: `decided` ends the
	// call with the user's decisions, `cancel` with why it stopped waiting.
	const waiting = new Map<string, { decided: (surface: Surface, text: string) => void; cancel: (why: string) => void }>();

	const remember = (details: unknown) => {
		if (!isObject(details) || typeof details.id !== "string" || details.id === "") return;
		seen.add(details.id);
		if (details.closed === true || !isObject(details.spec)) surfaces.delete(details.id);
		else surfaces.set(details.id, details as unknown as Surface);
	};

	const reconstruct = (ctx: ExtensionContext) => {
		for (const wait of [...waiting.values()]) wait.cancel("The session changed while waiting for the user's decisions.");
		surfaces.clear();
		pendingContext.clear();
		seen.clear();
		for (const entry of ctx.sessionManager.getBranch()) {
			if (entry.type === "message" && entry.message.role === "toolResult" && entry.message.toolName === TOOL) {
				remember(entry.message.details);
			} else if (entry.type === "custom_message" && entry.customType === EVENT_MESSAGE) {
				remember(entry.details);
			}
		}
	};
	pi.on("session_start", async (_event, ctx) => {
		workspaceRoot = ctx.cwd;
		reconstruct(ctx);
	});
	pi.on("session_tree", async (_event, ctx) => {
		workspaceRoot = ctx.cwd;
		reconstruct(ctx);
	});
	// ui/update-model-context: the page's latest context reaches the model at
	// the next turn, once (MCP Apps: deferred, last update wins).
	pi.on("before_agent_start", async () => {
		if (pendingContext.size === 0) return;
		const shared = [...pendingContext.values()];
		pendingContext.clear();
		const content = shared
			.map(({ surface, change }) => `Context shared by the HTML UI "${surface.title}" (${surface.id}):\n${contextText(change)}`)
			.join("\n\n");
		return { message: { customType: CONTEXT_MESSAGE, content, display: false, details: { ids: shared.map((s) => s.surface.id) } } };
	});

	const existing = (id: string | undefined): Surface => {
		const surface = id === undefined ? undefined : surfaces.get(id);
		if (surface) return surface;
		const open = [...surfaces.keys()].join(", ") || "none";
		throw new Error(`gentle_ui: no open UI with id "${id ?? ""}" (open: ${open})`);
	};

	const freshId = (wanted: string | undefined): string => {
		if (wanted !== undefined && SLUG.test(wanted) && !surfaces.has(wanted)) return wanted;
		let highest = 0;
		for (const id of seen) highest = Math.max(highest, Number(/^ui-(\d+)$/.exec(id)?.[1] ?? 0));
		return `ui-${highest + 1}`;
	};

	pi.registerTool<typeof Params, Details>({
		name: TOOL,
		label: "Gentle UI",
		description:
			"Show an interactive UI in the user's Gentle Shell app conversation (a diagram, table, form, checklist, kanban board, chart, change review or an html page), " +
			"update it (spec replaces, patch merges) or close it. The user can edit or use it (charts are read-only); the result summarizes its current state.",
		promptSnippet: "Show, update or close an interactive diagram, table, form, checklist, kanban board, chart, change review or html page the user can edit in the app",
		promptGuidelines: [
			"Use gentle_ui when the user should see or act on structured content, picking the kind by what they will do: diagram for architecture, data flow and dependencies; table for rows to compare, select or correct; form to ask several structured questions at once; checklist for plans and TODOs they check off or reorder; kanban for work items across stages; chart for numbers (read-only). Don't use gentle_ui for an answer that fits in a short reply.",
			"The user is in the Gentle Shell desktop app, where a gentle_ui diagram appears right in the conversation and they can move, connect and correct it. When they ask to see, map, draw or plan an architecture, system or flow, use gentle_ui, not gentle_design: gentle_design is for HTML design canvases (screens, mockups, visual directions) opened in a browser.",
			"To change a UI already shown, call gentle_ui with action update and a small patch instead of a new show; ids are stable slugs (api, orders-db) reused across updates.",
			`Whenever a gentle_ui item corresponds to code (a diagram node for a module, a table row for a function or test, a checklist item or kanban card for a change), attach ref: { path (repo-relative), line?, endLine? } so the user can open it, e.g. {"id":"auth","label":"Auth service","ref":{"path":"src/auth/mod.rs","line":42}}; paths outside the workspace are rejected.`,
			`The user may insert a saved UI (a "${EVENT_MESSAGE}" message saying so) or ask you to add one to the project's docs; treat inserted UIs like your own (update them by id).`,
			"In gentle_ui specs omit x/y and let auto-layout place nodes; keep labels short and put details in description or tech.",
			`The user's edits to a gentle_ui UI arrive as "${EVENT_MESSAGE}" messages with the full current spec; they are authoritative, so build on them and don't undo them.`,
			"A gentle_ui form's answers arrive once, when the user submits (op submit); wait for them instead of guessing. To ask again or change the questions, update the form: an update reopens it.",
			`gentle_ui example: ${EXAMPLE}`,
			...KIND_EXAMPLES,
			"A gentle_ui checklist with run: true is a plan the user runs: they skip and reorder steps and press Run or Run next, and a gentle-ui.event tells you which step to do next. Before a step, update it {items:[{id, status:\"running\"}]}; after it, update it to done or failed (with a note). After Run (all), go on through the pending steps yourself; after Run next, stop after that step; when the user stops the plan, start no new step.",
			"A gentle_ui kanban column with starts: true dispatches work: when the user drags a card into it, a gentle-ui.event asks you to dispatch a subagent for that card (card.agent {name, task} says which and what). Dispatch it with your own subagent tool, then update the card with agentId set to the subagent's id: the card then shows its live status, step and cost, and the user can stop it there.",
			"gentle_ui tables, charts and checklists can be live: spec.live {source: jobs|subagents|git:status|git:log|todo} or {command: {cmd, cwd?, parse}, interval} with a map of JSONPath-lite picks. The app reads the data itself and keeps the UI current (commands only after the user allows them on the card), so don't push rows or values for a live UI; use it for dashboards of background jobs, subagents, git state or a test command's JSON.",
			"Use gentle_ui kind html only for custom interactive visuals, simulations, calculators or working prototypes no other kind fits (design screens and mockups stay in gentle_design). The page is an MCP App: self-contained (inline JS/CSS, no network unless spec.csp lists origins), put changing values in spec.data and update them with patch {data}.",
			"A gentle_ui html page talks JSON-RPC 2.0 over window.parent.postMessage(msg, \"*\"): request ui/initialize {protocolVersion:\"2026-01-26\",appInfo:{name,version},appCapabilities:{}} (result.hostContext.styles.variables: theme CSS vars like --color-background-primary, --color-text-primary, --color-ring-primary, --font-sans; apply them), then notify ui/notifications/initialized; it receives ui/notifications/tool-result {structuredContent: spec.data}. Request ui/message {role:\"user\",content:[{type:\"text\",text}]} to message you, ui/update-model-context {content|structuredContent} to share state, ui/open-link {url}; notify ui/notifications/size-changed {height}.",
			"Use gentle_ui kind review to let the user review proposed file changes before you write them: show it before large or risky edits, and before every edit when the user asked for review mode. Give per file hunks (lines \"+…\", \"-…\", \" …\" with oldStart/newStart) or before/after texts. Don't write any of those changes until the decisions arrive.",
			`gentle_ui review decisions arrive as a "${EVENT_MESSAGE}" message (or, with wait: true, as the tool result): apply ONLY the accepted hunks, exactly as proposed or as their comments ask; never apply rejected or undecided ones (revise a rejected hunk with update if its comment asks for something else). Then call update with patch {applied: [hunk ids]} so the card shows them applied.`,
			"Use gentle_ui review with wait: true when you can't continue without the decisions (the call returns them; the user can stop it); otherwise end your turn after showing the review.",
		],
		parameters: Params,
		executionMode: "sequential",
		// Some models send the spec or patch as a JSON string.
		prepareArguments(args) {
			if (!isObject(args)) return args as never;
			const parsed: Json = { ...args };
			for (const key of ["spec", "patch"]) {
				if (typeof parsed[key] !== "string") continue;
				try {
					parsed[key] = JSON.parse(parsed[key] as string);
				} catch {
					// Left as is: validation reports it.
				}
			}
			return parsed as never;
		},
		async execute(_toolCallId, params, signal, onUpdate, ctx) {
			workspaceRoot = ctx.cwd;
			if (params.action === "close") {
				const surface = existing(params.id);
				surfaces.delete(surface.id);
				pendingContext.delete(surface.id);
				const noun = (KINDS[surface.kind] ?? KINDS.diagram).noun;
				return {
					content: [{ type: "text", text: `Closed ${noun} "${surface.title}" (${surface.id}).` }],
					details: { id: surface.id, version: surface.version, kind: surface.kind, title: surface.title, spec: null, closed: true },
				};
			}
			const patch = params.patch;
			if (patch !== undefined && !isObject(patch)) throw new Error("patch must be an object");
			let surface: Surface;
			if (params.action === "show") {
				if (params.spec === undefined) throw new Error("gentle_ui show needs a spec");
				const kind = params.kind ?? "diagram";
				const def = KINDS[kind];
				const base = def.normalize(params.spec);
				const spec = patch === undefined ? base : def.patch(base, patch);
				surface = { id: freshId(params.id), version: 1, kind, title: params.title?.trim() || def.title, spec };
			} else {
				const current = existing(params.id);
				if (params.kind !== undefined && params.kind !== current.kind) {
					throw new Error(`gentle_ui: "${current.id}" is a ${current.kind}; show a new UI for a ${params.kind}`);
				}
				if (params.spec === undefined && patch === undefined) throw new Error("gentle_ui update needs a spec (replace) or a patch (merge)");
				const def = KINDS[current.kind] ?? KINDS.diagram;
				const base = params.spec === undefined ? current.spec : def.normalize(params.spec);
				surface = {
					...current,
					version: current.version + 1,
					title: params.title?.trim() || current.title,
					spec: patch === undefined ? base : def.patch(base, patch),
				};
			}
			surfaces.set(surface.id, surface);
			seen.add(surface.id);
			const text = (KINDS[surface.kind] ?? KINDS.diagram).summary(surface);
			if (params.wait !== true) return { content: [{ type: "text", text }], details: surface };
			if (surface.kind !== "review") throw new Error("gentle_ui: wait is only for kind review");
			// The card shows from this partial result while the call waits;
			// the user's decisions (`/gentle-ui:event`) end it, as do Stop,
			// a session change or the time limit (the review stays open).
			onUpdate?.({ content: [{ type: "text", text: `${text}\nWaiting for the user's decisions…` }], details: surface });
			const id = surface.id;
			waiting.get(id)?.cancel("A newer version of this review is waiting instead.");
			const { promise, resolve } = Promise.withResolvers<{ content: { type: "text"; text: string }[]; details: Surface }>();
			const finish = (details: Surface, result: string) => {
				clearTimeout(timer);
				signal?.removeEventListener("abort", onAbort);
				if (waiting.get(id) === wait) waiting.delete(id);
				resolve({ content: [{ type: "text", text: result }], details });
			};
			const stopped = (why: string) =>
				finish(surfaces.get(id) ?? surface, `${why} The review stays open: decisions the user sends later arrive as "${EVENT_MESSAGE}" messages; write none of its hunks before then.`);
			const onAbort = () => stopped("The user stopped waiting before deciding.");
			const timer = setTimeout(() => stopped(`No decisions after ${REVIEW_WAIT_MS / 60000} minutes.`), REVIEW_WAIT_MS);
			const wait = { decided: finish, cancel: stopped };
			waiting.set(id, wait);
			if (signal?.aborted) onAbort();
			else signal?.addEventListener("abort", onAbort, { once: true });
			return await promise;
		},
	});

	pi.registerCommand(EVENT_COMMAND, {
		description: "Gentle Shell app: what the user did in a gentle_ui UI (sent by the app)",
		handler: async (args, ctx) => {
			let event: Json;
			try {
				const parsed: unknown = JSON.parse(args.trim());
				if (!isObject(parsed)) throw new Error("not an object");
				event = parsed;
			} catch (error) {
				ctx.ui.notify(`gentle_ui: unreadable event (${(error as Error).message})`, "error");
				return;
			}
			const id = typeof event.id === "string" ? event.id : "";
			const baseVersion = typeof event.version === "number" ? event.version : undefined;
			// Every event is answered, whatever its kind: the app's card shows
			// Sending until then, and Sent, or the error with Retry.
			const answer = (outcome: { ok: true; current: number } | { ok: false; error: string }) => {
				ctx.ui.setWidget(OUTCOME_WIDGET, [JSON.stringify({ id, version: baseVersion ?? 0, ...outcome })]);
				ctx.ui.setWidget(OUTCOME_WIDGET, undefined);
			};
			workspaceRoot = ctx.cwd;
			// A saved template the user inserts: a new surface, as if shown.
			if (event.op === "insert") {
				const kind = typeof event.kind === "string" ? event.kind : "";
				const def = KINDS[kind];
				let spec: Json;
				try {
					if (!def) throw new Error(`unknown kind "${kind}"`);
					spec = def.normalize(event.spec);
				} catch (error) {
					answer({ ok: false, error: (error as Error).message });
					return;
				}
				const title = (typeof event.title === "string" && event.title.trim()) || def.title;
				const surface: Surface = { id: freshId(id || undefined), version: 1, kind, title, spec };
				surfaces.set(surface.id, surface);
				seen.add(surface.id);
				const template = typeof event.template === "string" && event.template.trim() !== "" ? event.template.trim() : title;
				const content = [
					`The user inserted the ${def.noun} "${title}" (${surface.id}, v1) from their saved template "${template}"; it is shown in the conversation and is yours to update like any gentle_ui surface.`,
					def.summary(surface),
				].join("\n");
				// Shown at once; the agent reads it with the next prompt (or now, mid-turn).
				pi.sendMessage(
					{ customType: EVENT_MESSAGE, display: true, content, details: { ...surface, inserted: true, template, changes: [], summary: [] } },
					ctx.isIdle() ? { triggerTurn: false } : { deliverAs: "steer" },
				);
				ctx.ui.setWidget(OUTCOME_WIDGET, [JSON.stringify({ id: surface.id, version: 0, ok: true, current: 1 })]);
				ctx.ui.setWidget(OUTCOME_WIDGET, undefined);
				return;
			}
			const current = surfaces.get(id);
			if (!current) {
				const open = [...surfaces.keys()].join(", ") || "none";
				answer({ ok: false, error: `The agent no longer has the UI "${id}" open (open: ${open}).` });
				return;
			}
			const def = KINDS[current.kind] ?? KINDS.diagram;
			// "Ask agent to add to docs": the surface as the app exported it.
			if (event.op === "docs") {
				const format = typeof event.format === "string" ? event.format : "Markdown";
				const fence = typeof event.fence === "string" ? event.fence : "";
				const text = typeof event.text === "string" ? event.text : undefined;
				const content = [
					`The user asks you to add the ${def.noun} "${current.title}" (${current.id}, v${current.version}) to the project's documentation, as ${format}: ` +
						"put it where it belongs under docs/ (extend a fitting file or create one), with a short introduction, and say where it went.",
					text === undefined ? def.summary(current) : fence === "" ? text : `\`\`\`${fence}\n${text.replace(/\n$/, "")}\n\`\`\``,
				].join("\n");
				pi.sendMessage(
					{ customType: EVENT_MESSAGE, display: true, content, details: { request: "docs", surface: current.id, kind: current.kind, title: current.title, version: current.version, format } },
					ctx.isIdle() ? { triggerTurn: true } : { deliverAs: "steer" },
				);
				answer({ ok: true, current: current.version });
				return;
			}
			const changes = Array.isArray(event.changes) ? event.changes : [];
			const html = current.kind === "html";
			if (html) {
				// A page doesn't edit its spec: what it says or shares is about the
				// page as the agent last sent it, so no version check applies.
				// Model context waits for the next turn instead of starting one.
				if (changes.length > 0 && changes.every((c) => isObject(c) && c.op === "context")) {
					pendingContext.set(current.id, { surface: current, change: changes[changes.length - 1] as Json });
					answer({ ok: true, current: current.version });
					return;
				}
			} else if (baseVersion !== undefined && baseVersion !== current.version) {
				answer({
					ok: false,
					error: `The agent changed this ${def.noun} meanwhile (now v${current.version}); review your edits on it and send again.`,
				});
				return;
			}
			let spec: Json;
			try {
				spec = html ? current.spec : def.normalize(event.spec);
			} catch (error) {
				answer({ ok: false, error: (error as Error).message });
				return;
			}
			const surface: Surface = { ...current, version: current.version + 1, spec };
			surfaces.set(surface.id, surface);
			const said = Array.isArray(event.summary) ? event.summary.filter((s): s is string => typeof s === "string") : [];
			const note = typeof event.note === "string" && event.note.trim() !== "" ? event.note.trim() : undefined;
			let content: string;
			if (html) {
				const shared = pendingContext.get(current.id);
				pendingContext.delete(current.id);
				const messages = changes.flatMap((c) => (isObject(c) && c.op === "message" && typeof c.text === "string" ? [c.text] : []));
				content = [
					`From the HTML UI "${surface.title}" (${surface.id}, v${surface.version}) the user says:`,
					...messages,
					...(shared ? [`Context the page shared: ${contextText(shared.change)}`] : []),
				].join("\n");
			} else if (current.kind === "review") {
				content = `${reviewDecisions(surface, changes, note)}\n${summaryReview(surface)}`;
				// A `wait: true` call gets the decisions as its result instead.
				const wait = waiting.get(surface.id);
				if (wait) {
					wait.decided(surface, content);
					answer({ ok: true, current: surface.version });
					return;
				}
			} else {
				content = [
					`The user edited the ${def.noun} "${surface.title}" (${surface.id}, now v${surface.version}) in the app:`,
					...said.map((line) => `- ${line}`),
					...(note ? [`Note from the user: ${note}`] : []),
					...(def.instructions?.(current, surface, changes) ?? []),
					`Changes: ${JSON.stringify(changes)}`,
					def.summary(surface),
				].join("\n");
			}
			pi.sendMessage(
				{ customType: EVENT_MESSAGE, display: true, content, details: { ...surface, changes, summary: said, note, baseVersion: current.version } },
				ctx.isIdle() ? { triggerTurn: true } : { deliverAs: "steer" },
			);
			answer({ ok: true, current: surface.version });
		},
	});
}
