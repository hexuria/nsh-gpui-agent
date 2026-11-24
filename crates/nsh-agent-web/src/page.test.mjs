import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
const source = await readFile(new URL("page.js", import.meta.url), "utf8");
function fakePlayer() {
  globalThis.location = { hash: "" };
  globalThis.sessionStorage = { getItem: () => null };
  vm.runInThisContext(source);
  const player = { views: new Map(), synthesize(event) { const result = globalThis.nshPageSteps.synth(event, this); if (result?.error) throw new Error(result.error); return result; }, testPage(query) { return globalThis.nshPageSteps[query.query](query, this); } };
  return { player };
}
function view(id, extra = {}) { return { _id: id, isConnected: true, parentElement: null, ...extra }; }

test("caret and word defaults refuse an actual email input without selection offsets", (t) => {
  const chrome = ["google-chrome", "chromium", "chrome"].find(
    (name) => spawnSync(name, ["--version"]).status === 0,
  );
  if (!chrome) return t.skip("Chrome is required for real input selection behavior");
  const profile = mkdtempSync(join(tmpdir(), "nsh-page-test-"));
  const script = `${source}\n
    try {
      const field = document.createElement("input");
      field.type = "email";
      field.value = "one@example.com";
      document.body.append(field);
      field.focus();
      if (field.selectionStart !== null) throw new Error("expected unavailable selection");
      const player = { views: new Map() };
      const cases = [
        ["ArrowLeft", {}], ["ArrowRight", {}], ["Home", {}], ["End", {}],
        ...["ArrowLeft", "ArrowRight", "Backspace", "Delete"].map(key => [key, { ctrl: true }]),
        ...["ArrowLeft", "ArrowRight", "Backspace"].map(key => [key, { meta: true }]),
      ];
      for (const [key, modifiers] of cases) {
        const result = nshPageSteps.synth({ event: "key", phase: "down", key, ...modifiers }, player);
        const expected = { dispatched: true, error: "key " + JSON.stringify(key) + " cannot be synthesized; use type for text" };
        if (JSON.stringify(result) !== JSON.stringify(expected)) throw new Error("unexpected response for " + key + ": " + JSON.stringify(result));
      }
      field.addEventListener("keydown", event => event.preventDefault());
      nshPageSteps.synth({ event: "key", phase: "down", key: "Home" }, player);
      if (field.value !== "one@example.com") throw new Error("email changed");
      document.body.textContent = "ok";
    } catch (error) { document.body.textContent = error.stack; }
  `;
  try {
    const html = `<body><script>${script}</script></body>`;
    const output = execFileSync(chrome, [
      "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
      `--user-data-dir=${profile}`, "--dump-dom", `data:text/html,${encodeURIComponent(html)}`,
    ], { encoding: "utf8", timeout: 30000, stdio: ["ignore", "pipe", "pipe"] });
    assert.ok(output.includes("<body>ok</body>"), output);
  } finally {
    rmSync(profile, { recursive: true, force: true });
  }
});

test("pairing consumes the fragment, survives reload and rejects malformed keys", () => {
  const key = "ab".repeat(16);
  const stored = new Map();
  const replacements = [];
  function load(hash, extra = {}) {
    const context = {
      location: { hash, pathname: "/app", search: "?x=1" },
      history: { state: null, replaceState: (...args) => replacements.push(args) },
      sessionStorage: { getItem: (k) => stored.get(k), setItem: (k, v) => stored.set(k, v) },
      ...extra,
    };
    vm.runInNewContext(source, context);
    return context;
  }
  assert.equal(load(`#route&gpui-agent=${key}`).nshHelloExtra, key);
  assert.equal(replacements[0][2], "/app?x=1#route");
  assert.equal(load("").nshHelloExtra, key);
  assert.equal(load("#gpui-agent=bad").nshHelloExtra, undefined);
  assert.equal(load(`#gpui-agent=${key}&gpui-agent=${key}`).nshHelloExtra, undefined);
  const owned = load("", { __nshAgentKey: key });
  assert.equal(owned.nshHelloExtra, key);
  assert.equal(owned.__nshAgentKey, undefined);
  assert.equal(load("", { __nshAgentKey: "bad" }).nshHelloExtra, undefined);
  stored.clear();
  assert.equal(load("").nshHelloExtra, undefined);
});

test("a refused page shows it and forgets the key; a paired page does not", () => {
  const key = "cd".repeat(16);
  class FakeSocket {
    constructor(url) {
      this.url = url;
      this.listeners = [];
    }
    addEventListener(type, listener) {
      if (type === "message") this.listeners.push(listener);
    }
    receive(data) {
      for (const listener of this.listeners) listener({ data });
    }
  }
  function load(stored) {
    const appended = [];
    const context = {
      location: { hash: "", pathname: "/", search: "" },
      history: { state: null, replaceState() {} },
      sessionStorage: {
        getItem: (k) => stored.get(k),
        setItem: (k, v) => stored.set(k, v),
        removeItem: (k) => stored.delete(k),
      },
      document: {
        body: { append: (node) => appended.push(node) },
        createElement: () => ({ style: {}, setAttribute(name, value) { this[name] = value; } }),
      },
      WebSocket: FakeSocket,
    };
    vm.runInNewContext(source, context);
    return { context, appended };
  }

  // The paired tab, and a reload of it: hellos that pair change nothing.
  const tab = new Map([["nsh-agent-key", key]]);
  const paired = load(tab);
  const socket = new paired.context.WebSocket("ws://127.0.0.1/ws");
  assert.equal(socket.url, "ws://127.0.0.1/ws");
  socket.receive(JSON.stringify({ type: "hello", observer: true, test: false }));
  socket.receive(new ArrayBuffer(1));
  socket.receive("not json");
  assert.equal(paired.context.nshHelloExtra, key);
  assert.equal(tab.get("nsh-agent-key"), key);
  assert.deepEqual(paired.appended, []);

  // A duplicate: same session storage, refused by the server.
  const copy = new Map(tab);
  const duplicate = load(copy);
  const refused = new duplicate.context.WebSocket("ws://127.0.0.1/ws");
  refused.receive(JSON.stringify({ type: "bundle", version: 1 }));
  assert.equal(duplicate.context.nshHelloExtra, key);
  refused.receive(JSON.stringify({ type: "hello", observer: false, test: false }));
  assert.equal(duplicate.context.nshHelloExtra, undefined);
  assert.equal(copy.has("nsh-agent-key"), false);
  assert.equal(tab.get("nsh-agent-key"), key, "the paired tab keeps its key");
  assert.equal(duplicate.appended.length, 1);
  assert.equal(duplicate.appended[0].role, "alert");
  assert.match(duplicate.appended[0].textContent, /another tab is already paired/);
  // Its next connections present no key, and show the refusal once.
  new duplicate.context.WebSocket("ws://127.0.0.1/ws")
    .receive(JSON.stringify({ type: "hello", observer: false }));
  assert.equal(duplicate.appended.length, 1);
  assert.equal(load(copy).context.nshHelloExtra, undefined);

  // A page without a key leaves the browser's WebSocket alone.
  assert.equal(load(new Map()).context.WebSocket, FakeSocket);
});

test("page focus questions track the real field and app focus", () => {
  const saved = globalThis.document;
  const { player } = fakePlayer();
  const owner = view(7);
  const field = { isConnected: true, parentElement: owner, focus: () => { document.activeElement = field; } };
  owner._field = field;
  player.views.set(7, owner);
  globalThis.document = { activeElement: null, hasFocus: () => true };
  try {
    assert.equal(player.testPage({ query: "focused" }), null);
    assert.equal(player.testPage({ query: "focus", id: 999 }), false);
    assert.equal(player.testPage({ query: "focus", id: 7 }), true);
    assert.equal(player.testPage({ query: "focused" }), 7);
    assert.equal(player.testPage({ query: "app_focused" }), true);
    field.isConnected = false;
    assert.equal(player.testPage({ query: "focus", id: 7 }), false);
  } finally { globalThis.document = saved; }
});

test("synthetic pointer clicks focus inputs but drags and prevented presses do not", () => {
  const saved = Object.fromEntries(["document", "PointerEvent", "MouseEvent", "HTMLInputElement", "HTMLTextAreaElement"].map((name) => [name, globalThis[name]]));
  const { player } = fakePlayer();
  class Event { constructor(type, init) { Object.assign(this, init, { type, defaultPrevented: false }); } }
  class Input {
    value = "text";
    type = "text";
    prevent = false;
    dispatchEvent(event) { if (this.prevent && event.type === "pointerdown") event.defaultPrevented = true; }
    closest() { return this; }
    focus() { document.activeElement = this; }
    setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; }
  }
  const field = new Input();
  globalThis.PointerEvent = Event;
  globalThis.MouseEvent = Event;
  globalThis.HTMLInputElement = Input;
  globalThis.HTMLTextAreaElement = class {};
  globalThis.document = { activeElement: null, elementFromPoint: () => field };
  const pointer = (phase, x) => player.synthesize({ event: "pointer", phase, x, y: 0, buttons: phase === "up" ? 0 : 1 });
  try {
    pointer("move", 0); pointer("down", 0); pointer("up", 0);
    assert.equal(document.activeElement, field);
    assert.equal(field.selectionStart, 4);
    document.activeElement = null;
    pointer("down", 0); pointer("move", 5); pointer("up", 5);
    assert.equal(document.activeElement, null);
    field.prevent = true;
    pointer("down", 0); pointer("up", 0);
    assert.equal(document.activeElement, null);
  } finally { Object.assign(globalThis, saved); }
});

test("synthetic wheel, history and invalid input use the page-step handler", () => {
  const saved = { document: globalThis.document, WheelEvent: globalThis.WheelEvent, history: globalThis.history };
  const { player } = fakePlayer();
  const events = [];
  let backs = 0;
  globalThis.WheelEvent = class { static DOM_DELTA_PIXEL = 0; constructor(type, data) { Object.assign(this, data, { type }); } };
  globalThis.document = { elementFromPoint: () => ({ dispatchEvent: (e) => events.push(e) }) };
  globalThis.history = { back: () => { backs++; } };
  try {
    assert.equal(player.synthesize({ event: "wheel", x: 4, y: 5, dx: 6, dy: 7 }), null);
    assert.equal(events[0].deltaY, 7);
    assert.equal(events[0].clientX, 4);
    player.synthesize({ event: "back" });
    assert.equal(backs, 1);
    assert.throws(() => player.synthesize({ event: "unknown" }), /unknown synthetic event/);
    document.elementFromPoint = () => null;
    assert.throws(() => player.synthesize({ event: "wheel" }), /no element/);
    assert.throws(() => player.synthesize({ event: "pointer" }), /no element/);
  } finally { Object.assign(globalThis, saved); }
});
test("synthetic text refuses read-only and disabled inputs before editing", () => {
  class Input {}
  class Textarea {}
  const { player } = fakePlayer();
  const view = { _id: 7, _props: { disabled: false } };
  const field = Object.assign(new Input(), {
    disabled: false,
    parentElement: view,
    readOnly: true,
    value: "before",
  });
  view._field = field;
  player.views = new Map([[7, view]]);
  const commands = [];
  const saved = {
    document: globalThis.document,
    HTMLInputElement: globalThis.HTMLInputElement,
    HTMLTextAreaElement: globalThis.HTMLTextAreaElement,
  };
  globalThis.HTMLInputElement = Input;
  globalThis.HTMLTextAreaElement = Textarea;
  globalThis.document = {
    activeElement: field,
    execCommand(command) {
      commands.push(command);
      return false;
    },
  };
  try {
    assert.throws(() => player.testPage({ query: "editable" }), { message: "node 7 is read-only" });
    assert.throws(
      () => player.synthesize({ event: "insert_text", text: "after" }),
      { message: "node 7 is read-only" },
    );
    field.readOnly = false;
    view._props.disabled = true;
    assert.throws(() => player.testPage({ query: "editable" }), { message: "node 7 is disabled" });
    assert.throws(
      () => player.synthesize({ event: "insert_text", text: "after" }),
      { message: "node 7 is disabled" },
    );
    assert.deepEqual(commands, []);
    assert.equal(field.value, "before");
    view._props.disabled = false;
    assert.equal(player.testPage({ query: "editable" }), null);
  } finally {
    for (const [name, value] of Object.entries(saved)) {
      if (value === undefined) delete globalThis[name];
      else globalThis[name] = value;
    }
  }
});

test("synthetic focus fails when the requested view does not take focus", () => {
  const { player } = fakePlayer();
  const previous = {};
  const target = {
    isConnected: true,
    focus() {},
  };
  player.views = new Map([[7, target]]);
  const savedDocument = globalThis.document;
  globalThis.document = { activeElement: previous };
  try {
    assert.throws(
      () => player.synthesize({ event: "focus", node: 7 }),
      { message: "node 7 cannot receive native focus" },
    );
    assert.equal(document.activeElement, previous);

    target.focus = () => { document.activeElement = target; };
    assert.doesNotThrow(() => player.synthesize({ event: "focus", node: 7 }));
    assert.equal(document.activeElement, target);
  } finally {
    if (savedDocument === undefined) delete globalThis.document;
    else globalThis.document = savedDocument;
  }
});

test("a GPUI agent's page never answers a masked input's text", () => {
  const { player } = fakePlayer();
  const secret = view(2, { _field: { type: "password", value: "hunter2" } });
  const plain = view(3, { _field: { type: "text", value: "hello" } });
  player.views = new Map([[2, secret], [3, plain]]);
  // A test session reads what the test typed.

  player.agent = true;
  assert.equal(player.testPage({ query: "input_text", id: 2 }), "•••••••");
  assert.equal(player.testPage({ query: "input_text", id: 3 }), "hello");
});

test("a hit is the view around the element the browser finds", () => {
  const { player } = fakePlayer();
  const root = view(1);
  const button = view(4, { parentElement: root });
  // A view's border overlay, and a deleted view still leaving the page.
  const border = { parentElement: button };
  const leaving = view(6, { parentElement: root });
  player.views = new Map([[1, root], [4, button]]);
  let found = border;
  globalThis.document = { elementFromPoint: (x, y) => (x === 5 && y === 6 ? found : null) };
  try {
    assert.equal(player.testPage({ query: "hit_at", x: 5, y: 6 }), 4);
    found = leaving;
    assert.equal(player.testPage({ query: "hit_at", x: 5, y: 6 }), 1);
    assert.equal(player.testPage({ query: "hit_at", x: 0, y: 0 }), null);
  } finally {
    delete globalThis.document;
  }
});

test("synthetic keys dispatch to handlers before refusing skipped defaults", () => {
  class Input {
    dispatchEvent(event) {
      dispatched.push([event.type, event.key]);
      if (event.type === "keydown" && ["Enter", "Escape"].includes(event.key)) {
        event.preventDefault();
      }
    }
  }
  class Textarea {}
  class KeyEvent {
    constructor(type, props) {
      this.type = type;
      Object.assign(this, props);
      this.defaultPrevented = false;
    }
    preventDefault() {
      this.defaultPrevented = true;
    }
  }
  const { player } = fakePlayer();
  const dispatched = [];
  const commands = [];
  const field = new Input();
  Object.assign(field, {
    isContentEditable: false,
    select() {},
    value: "",
  });
  const saved = {
    document: globalThis.document,
    HTMLInputElement: globalThis.HTMLInputElement,
    HTMLTextAreaElement: globalThis.HTMLTextAreaElement,
    KeyboardEvent: globalThis.KeyboardEvent,
  };
  globalThis.HTMLInputElement = Input;
  globalThis.HTMLTextAreaElement = Textarea;
  globalThis.KeyboardEvent = KeyEvent;
  globalThis.document = {
    activeElement: field,
    execCommand(command) {
      commands.push(command);
      return true;
    },
  };
  try {
    const synthesize = (key, phase = "down", modifiers = {}) => player.synthesize({
      event: "key",
      phase,
      key,
      code: key,
      shift: false,
      ctrl: false,
      alt: false,
      meta: false,
      ...modifiers,
    });
    assert.doesNotThrow(() => synthesize("Enter"));
    assert.doesNotThrow(() => synthesize("Escape"));
    for (const key of ["a", "Tab", "ArrowUp", "ArrowDown", "PageUp", "PageDown"]) {
      assert.throws(
        () => synthesize(key),
        { message: `key ${JSON.stringify(key)} cannot be synthesized; use type for text` },
      );
    }
    assert.doesNotThrow(() => synthesize("F1"));
    assert.doesNotThrow(() => synthesize("a", "up"));
    assert.doesNotThrow(() => synthesize("Backspace"));
    assert.deepEqual(dispatched, [
      ["keydown", "Enter"],
      ["keydown", "Escape"],
      ["keydown", "a"],
      ["keydown", "Tab"],
      ["keydown", "ArrowUp"],
      ["keydown", "ArrowDown"],
      ["keydown", "PageUp"],
      ["keydown", "PageDown"],
      ["keydown", "F1"],
      ["keyup", "a"],
      ["keydown", "Backspace"],
    ]);
    assert.deepEqual(commands, ["delete"]);
  } finally {
    for (const [name, value] of Object.entries(saved)) {
      if (value === undefined) delete globalThis[name];
      else globalThis[name] = value;
    }
  }
});

test("synthetic modified shortcuts refuse defaults they do not apply", () => {
  class Input {
    constructor() {
      this.value = "one two";
      this.selectionStart = this.value.length;
      this.selectionEnd = this.value.length;
      this.selectionDirection = "none";
      this.pageHandlesCtrlZ = false;
    }
    dispatchEvent(event) {
      dispatched.push([event.type, event.key, event.ctrlKey, event.altKey, event.metaKey]);
      if (this.pageHandlesCtrlZ && event.key === "z" && event.ctrlKey) {
        event.preventDefault();
      }
    }
    select() {
      this.setSelectionRange(0, this.value.length);
    }
    setSelectionRange(start, end, direction = "none") {
      this.selectionStart = start;
      this.selectionEnd = end;
      this.selectionDirection = direction;
    }
  }
  class Textarea {}
  class KeyEvent {
    constructor(type, props) {
      this.type = type;
      Object.assign(this, props);
      this.defaultPrevented = false;
    }
    preventDefault() {
      this.defaultPrevented = true;
    }
  }
  const { player } = fakePlayer();
  const dispatched = [];
  const commands = [];
  const field = new Input();
  const saved = {
    document: globalThis.document,
    HTMLInputElement: globalThis.HTMLInputElement,
    HTMLTextAreaElement: globalThis.HTMLTextAreaElement,
    KeyboardEvent: globalThis.KeyboardEvent,
  };
  const savedNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  globalThis.HTMLInputElement = Input;
  globalThis.HTMLTextAreaElement = Textarea;
  globalThis.KeyboardEvent = KeyEvent;
  globalThis.document = {
    activeElement: field,
    execCommand(command) {
      commands.push(command);
      if (command === "delete") {
        const start = field.selectionStart;
        const end = field.selectionEnd;
        field.value = field.value.slice(0, start) + field.value.slice(end);
        field.setSelectionRange(start, start);
      }
      return true;
    },
  };
  const setPlatform = (platform) => {
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: { platform },
    });
  };
  try {
    const synthesize = (key, modifiers = {}) => player.synthesize({
      event: "key",
      phase: "down",
      key,
      code: key,
      shift: false,
      ctrl: false,
      alt: false,
      meta: false,
      ...modifiers,
    });
    const unsupported = (key, modifiers) => assert.throws(
      () => synthesize(key, modifiers),
      { message: `key ${JSON.stringify(key)} cannot be synthesized; use type for text` },
    );

    unsupported("Home", { ctrl: true });
    unsupported("z", { ctrl: true });
    unsupported("z", { meta: true });
    assert.doesNotThrow(() => synthesize("a", { ctrl: true }));
    assert.deepEqual([field.selectionStart, field.selectionEnd], [0, field.value.length]);
    assert.doesNotThrow(() => synthesize("Control", { ctrl: true }));

    for (const [platform, modifiers] of [
      ["Linux x64", { ctrl: true }],
      ["MacIntel", { alt: true }],
    ]) {
      setPlatform(platform);
      field.value = "one two";
      field.setSelectionRange(field.value.length, field.value.length);
      commands.length = 0;
      assert.doesNotThrow(() => synthesize("Backspace", modifiers));
      assert.equal(field.value, "one ");
      assert.deepEqual(commands, ["delete"]);
    }

    field.pageHandlesCtrlZ = true;
    assert.doesNotThrow(() => synthesize("z", { ctrl: true }));
    assert.deepEqual(dispatched.slice(0, 3), [
      ["keydown", "Home", true, false, false],
      ["keydown", "z", true, false, false],
      ["keydown", "z", false, false, true],
    ]);
  } finally {
    for (const [name, value] of Object.entries(saved)) {
      if (value === undefined) delete globalThis[name];
      else globalThis[name] = value;
    }
    if (savedNavigator) Object.defineProperty(globalThis, "navigator", savedNavigator);
    else delete globalThis.navigator;
  }
});
