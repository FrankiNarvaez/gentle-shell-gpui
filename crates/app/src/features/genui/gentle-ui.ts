// Gentle Shell GPUI: generative UI. The `gentle_ui` tool shows an interactive
// diagram in the desktop app's conversation; the user edits it natively and the
// app sends the edits back with `/gentle-ui:event {json}`. State lives only in
// the session (tool result details and gentle-ui.event messages), so branches,
// forks and resumes rebuild it. Written by the app; edits here are overwritten.
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { StringEnum } from "@earendil-works/pi-ai";
import { Type } from "typebox";

const TOOL = "gentle_ui";
const EVENT_MESSAGE = "gentle-ui.event";
const EVENT_COMMAND = "gentle-ui:event";

const NODE_KINDS = ["service", "database", "queue", "cache", "client", "external", "storage", "function", "note"];
const STATUSES = ["planned", "existing", "deprecated", "risk"];
const EDGE_STYLES = ["sync", "async", "data", "dependency"];
const MAX = { nodes: 200, edges: 400, groups: 50, id: 64, label: 80, text: 1000, errors: 10, listed: 60 };
const SLUG = /^[a-z0-9][a-z0-9-_]{0,63}$/;

type Json = Record<string, unknown>;
interface Group { id: string; label: string; description?: string }
interface Node {
	id: string; label: string; kind: string; group?: string; description?: string;
	tech?: string; status?: string; x?: number; y?: number;
}
interface Edge { id: string; from: string; to: string; label?: string; style: string }
interface Spec { direction: "LR" | "TB"; groups: Group[]; nodes: Node[]; edges: Edge[] }
interface Surface { id: string; version: number; kind: "diagram"; title: string; spec: Spec }
type Details = Surface | { id: string; version: number; kind: "diagram"; title: string; spec: null; closed: true };

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
	const { spec } = surface;
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
	lines.push(`The user can move, rename, connect and delete parts; their edits arrive as "${EVENT_MESSAGE}" messages.`);
	return lines.join("\n");
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

const Params = Type.Object({
	action: StringEnum(["show", "update", "close"] as const),
	id: Type.Optional(Type.String({ description: "Stable slug for the UI (show) or the id to update/close" })),
	title: Type.Optional(Type.String({ description: "Short title, default \"Diagram\"" })),
	kind: Type.Optional(StringEnum(["diagram"] as const)),
	spec: Type.Optional(Type.Object({}, {
		additionalProperties: true,
		description:
			"Full diagram: { direction?: LR|TB, groups?: [{id,label,description?}], nodes: [{id,label,kind?,group?,description?,tech?,status?}], edges?: [{id?,from,to,label?,style?}] }. " +
			`Node kind: ${NODE_KINDS.join("|")}. Status: ${STATUSES.join("|")}. Edge style: ${EDGE_STYLES.join("|")}.`,
	})),
	patch: Type.Optional(Type.Object({}, {
		additionalProperties: true,
		description:
			"Changes for update: { nodes?: [partial nodes, upsert by id; null clears a field], edges?: [upsert by id or from+to], groups?: [upsert by id], remove?: { nodes?: [ids], edges?: [ids], groups?: [ids] }, direction? }",
	})),
});

export default function gentleUi(pi: ExtensionAPI) {
	const surfaces = new Map<string, Surface>();
	// Every id seen on the branch, closed ones included, so `ui-<n>` never repeats.
	const seen = new Set<string>();

	const remember = (details: unknown) => {
		if (!isObject(details) || typeof details.id !== "string" || details.id === "") return;
		seen.add(details.id);
		if (details.closed === true || !isObject(details.spec)) surfaces.delete(details.id);
		else surfaces.set(details.id, details as unknown as Surface);
	};

	const reconstruct = (ctx: ExtensionContext) => {
		surfaces.clear();
		seen.clear();
		for (const entry of ctx.sessionManager.getBranch()) {
			if (entry.type === "message" && entry.message.role === "toolResult" && entry.message.toolName === TOOL) {
				remember(entry.message.details);
			} else if (entry.type === "custom_message" && entry.customType === EVENT_MESSAGE) {
				remember(entry.details);
			}
		}
	};
	pi.on("session_start", async (_event, ctx) => reconstruct(ctx));
	pi.on("session_tree", async (_event, ctx) => reconstruct(ctx));

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
			"Show an interactive diagram (architecture, data flow, dependencies) in the user's Gentle Shell app conversation, " +
			"update it (spec replaces, patch merges) or close it. The user can edit it; the result summarizes the current diagram.",
		promptSnippet: "Show, update or close an interactive architecture/data-flow diagram the user can edit in the app",
		promptGuidelines: [
			"Use gentle_ui to show architecture, data flow, dependencies or plans with many interacting parts as a diagram; don't use gentle_ui for simple lists or linear steps.",
			"The user is in the Gentle Shell desktop app, where a gentle_ui diagram appears right in the conversation and they can move, connect and correct it. When they ask to see, map, draw or plan an architecture, system or flow, use gentle_ui, not gentle_design: gentle_design is for HTML design canvases (screens, mockups, visual directions) opened in a browser.",
			"To change a diagram already shown, call gentle_ui with action update and a small patch instead of a new show; ids are stable slugs (api, orders-db) reused across updates.",
			"In gentle_ui specs omit x/y and let auto-layout place nodes; keep labels short and put details in description or tech.",
			`The user's edits to a gentle_ui diagram arrive as "${EVENT_MESSAGE}" messages with the full current spec; they are authoritative, so build on them and don't undo them.`,
			`gentle_ui example: ${EXAMPLE}`,
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
		async execute(_toolCallId, params) {
			if (params.action === "close") {
				const surface = existing(params.id);
				surfaces.delete(surface.id);
				return {
					content: [{ type: "text", text: `Closed diagram "${surface.title}" (${surface.id}).` }],
					details: { id: surface.id, version: surface.version, kind: "diagram", title: surface.title, spec: null, closed: true },
				};
			}
			let surface: Surface;
			if (params.action === "show") {
				if (params.spec === undefined) throw new Error("gentle_ui show needs a spec");
				const spec = applyPatch(normalize(params.spec), params.patch);
				surface = { id: freshId(params.id), version: 1, kind: "diagram", title: params.title?.trim() || "Diagram", spec };
			} else {
				const current = existing(params.id);
				if (params.spec === undefined && params.patch === undefined) throw new Error("gentle_ui update needs a spec (replace) or a patch (merge)");
				const base = params.spec === undefined ? current.spec : normalize(params.spec);
				surface = {
					...current,
					version: current.version + 1,
					title: params.title?.trim() || current.title,
					spec: applyPatch(base, params.patch),
				};
			}
			surfaces.set(surface.id, surface);
			seen.add(surface.id);
			return { content: [{ type: "text", text: summary(surface) }], details: surface };
		},
	});

	pi.registerCommand(EVENT_COMMAND, {
		description: "Gentle Shell app: the user's edits to a gentle_ui diagram (sent by the app)",
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
			const current = typeof event.id === "string" ? surfaces.get(event.id) : undefined;
			if (!current) {
				ctx.ui.notify(`gentle_ui: no UI with id ${JSON.stringify(event.id ?? null)}`, "error");
				return;
			}
			let spec: Spec;
			try {
				spec = normalize(event.spec);
			} catch (error) {
				ctx.ui.notify(`gentle_ui: ${(error as Error).message}`, "error");
				return;
			}
			const surface: Surface = { ...current, version: current.version + 1, spec };
			surfaces.set(surface.id, surface);
			const changes = Array.isArray(event.changes) ? event.changes : [];
			const said = Array.isArray(event.summary) ? event.summary.filter((s): s is string => typeof s === "string") : [];
			const note = typeof event.note === "string" && event.note.trim() !== "" ? event.note.trim() : undefined;
			const baseVersion = typeof event.version === "number" ? event.version : current.version;
			const content = [
				`The user edited the diagram "${surface.title}" (${surface.id}, now v${surface.version}) in the app:`,
				...said.map((line) => `- ${line}`),
				...(note ? [`Note from the user: ${note}`] : []),
				`Changes: ${JSON.stringify(changes)}`,
				summary(surface),
			].join("\n");
			pi.sendMessage(
				{ customType: EVENT_MESSAGE, display: true, content, details: { ...surface, changes, summary: said, note, baseVersion } },
				ctx.isIdle() ? { triggerTurn: true } : { deliverAs: "steer" },
			);
		},
	});
}
