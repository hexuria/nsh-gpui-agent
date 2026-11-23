// Web page pairing, scene questions and synthetic input, ported from NSH buwiz.
(() => {
const SYNTHETIC_CLICK_SLOP = 4;
let graphemes;
let wordSegmenter;

function inputSelection(input) {
  try {
    const start = input.selectionStart;
    if (start === null) return null;
    return {
      start,
      end: input.selectionEnd ?? start,
      backward: input.selectionDirection === "backward",
    };
  } catch {
    return null;
  }
}

function setInputSelection(input, start, end, direction = "none") {
  try {
    input.setSelectionRange(start, end, direction);
    return true;
  } catch {
    return false;
  }
}

function lineBoundary(value, position, direction) {
  if (direction === "backward") {
    return position === 0 ? 0 : value.lastIndexOf("\n", position - 1) + 1;
  }
  const nextLine = value.indexOf("\n", position);
  return nextLine < 0 ? value.length : nextLine;
}

function wordBoundary(value, position, direction) {
  wordSegmenter ??= typeof Intl !== "undefined" && typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "word" })
    : null;
  if (!wordSegmenter) return position;
  if (direction === "backward") {
    let boundary = 0;
    for (const part of wordSegmenter.segment(value)) {
      if (part.index >= position) break;
      if (part.isWordLike) boundary = part.index;
    }
    return boundary;
  }
  for (const part of wordSegmenter.segment(value)) {
    const end = part.index + part.segment.length;
    if (part.isWordLike && end > position) return end;
  }
  return value.length;
}

function graphemeBoundary(value, position, direction) {
  graphemes ??= typeof Intl !== "undefined" && typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;
  if (graphemes) {
    if (direction === "backward") {
      let previous = 0;
      for (const part of graphemes.segment(value)) {
        const end = part.index + part.segment.length;
        if (position <= end) return part.index;
        previous = part.index;
      }
      return previous;
    }
    for (const part of graphemes.segment(value)) {
      const end = part.index + part.segment.length;
      if (end > position) return end;
    }
    return value.length;
  }
  if (direction === "backward") {
    if (position === 0) return 0;
    let start = position - 1;
    const code = value.charCodeAt(start);
    if (code >= 0xdc00 && code <= 0xdfff && start > 0) start--;
    return start;
  }
  if (position >= value.length) return value.length;
  const code = value.codePointAt(position);
  return position + (code > 0xffff ? 2 : 1);
}


function synthesize(event) {
    if (event.event === "pointer") {
      const target = document.elementFromPoint(event.x, event.y);
      if (!target) throw new Error("no element at the synthetic pointer position");
      const names = {
        down: ["pointerdown", "mousedown"],
        move: ["pointermove", "mousemove"],
        up: ["pointerup", "mouseup"],
      }[event.phase];
      if (!names) throw new Error(`unknown pointer phase ${event.phase}`);
      if (event.phase === "move" && this.syntheticPointerTarget !== target) {
        const previous = this.syntheticPointerTarget;
        const path = (element) => {
          const ancestors = [];
          for (let parent = element; parent; parent = parent.parentElement) ancestors.push(parent);
          return ancestors;
        };
        const previousPath = previous ? path(previous) : [];
        const targetPath = path(target);
        const boundary = (element, pointerName, mouseName, relatedTarget, bubbles) => {
          const init = {
            button: -1, buttons: event.buttons, clientX: event.x, clientY: event.y, relatedTarget,
            bubbles, composed: true, cancelable: bubbles,
          };
          element.dispatchEvent(new PointerEvent(pointerName, {
            ...init, pointerType: "mouse", pointerId: 1,
          }));
          element.dispatchEvent(new MouseEvent(mouseName, init));
        };
        if (previous) {
          boundary(previous, "pointerout", "mouseout", target, true);
          for (const element of previousPath.filter((item) => !targetPath.includes(item))) {
            boundary(element, "pointerleave", "mouseleave", target, false);
          }
        }
        boundary(target, "pointerover", "mouseover", previous, true);
        for (const element of targetPath.filter((item) => !previousPath.includes(item)).reverse()) {
          boundary(element, "pointerenter", "mouseenter", previous, false);
        }
        this.syntheticPointerTarget = target;
      }
      const pointer = new PointerEvent(names[0], {
        pointerType: "mouse", pointerId: 1, button: event.phase === "move" ? -1 : 0, buttons: event.buttons,
        clientX: event.x, clientY: event.y, bubbles: true, composed: true, cancelable: true,
      });
      const mouse = new MouseEvent(names[1], {
        button: event.phase === "move" ? -1 : 0, buttons: event.buttons, clientX: event.x, clientY: event.y,
        bubbles: true, composed: true, cancelable: true,
      });
      target.dispatchEvent(pointer);
      target.dispatchEvent(mouse);
      if (event.phase === "down") {
        this.syntheticPointerDown = {
          x: event.x,
          y: event.y,
          moved: false,
          prevented: pointer.defaultPrevented || mouse.defaultPrevented,
        };
      } else if (event.phase === "move" && this.syntheticPointerDown) {
        if (
          Math.hypot(event.x - this.syntheticPointerDown.x, event.y - this.syntheticPointerDown.y)
          > SYNTHETIC_CLICK_SLOP
        ) {
          this.syntheticPointerDown.moved = true;
        }
      } else if (event.phase === "up") {
        const down = this.syntheticPointerDown;
        this.syntheticPointerDown = null;
        if (
          down && !down.moved && !down.prevented && !pointer.defaultPrevented && !mouse.defaultPrevented
          && Math.hypot(event.x - down.x, event.y - down.y) <= SYNTHETIC_CLICK_SLOP
        ) {
          const focusable = target.closest("input, textarea, select, [contenteditable], [tabindex], button, a[href]");
          if (focusable && document.activeElement !== focusable) focusable.focus({ preventScroll: true });
          if (
            document.activeElement === focusable
            && (focusable instanceof HTMLInputElement || focusable instanceof HTMLTextAreaElement)
          ) {
            const end = focusable.value.length;
            const canSetCaret = focusable instanceof HTMLTextAreaElement
              || ["text", "search", "tel", "url", "password"].includes(focusable.type);
            if (canSetCaret) focusable.setSelectionRange(end, end);
          }
        }
      }
      return;
    }
    if (event.event === "wheel") {
      const target = document.elementFromPoint(event.x, event.y);
      if (!target) throw new Error("no element at the synthetic wheel position");
      target.dispatchEvent(new WheelEvent("wheel", {
        deltaX: event.dx, deltaY: event.dy, deltaMode: WheelEvent.DOM_DELTA_PIXEL,
        clientX: event.x, clientY: event.y, bubbles: true, composed: true,
      }));
      return;
    }
    if (event.event === "key") {
      const target = document.activeElement;
      if (!target) return;
      const input = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
      const contenteditable = target?.isContentEditable;
      const key = event.key;
      const macOS = globalThis.navigator?.platform?.startsWith("Mac") ?? false;
      const wordModifier = macOS
        ? event.alt && !event.ctrl && !event.meta
        : event.ctrl && !event.alt && !event.meta;
      const lineModifier = macOS && event.meta && !event.ctrl && !event.alt;
      const wordAction = wordModifier
        && ["Backspace", "Delete", "ArrowLeft", "ArrowRight"].includes(key);
      const lineAction = lineModifier && ["Backspace", "ArrowLeft", "ArrowRight"].includes(key);
      const caretKeys = ["Backspace", "Delete", "ArrowLeft", "ArrowRight", "Home", "End"];
      const editable = input || contenteditable;
      const selectAll = editable && (event.ctrl || event.meta) && key.toLowerCase() === "a";
      const enter = key === "Enter" && (target instanceof HTMLTextAreaElement || contenteditable);
      const caret = editable
        && caretKeys.includes(key)
        && (!(event.ctrl || event.alt || event.meta) || wordAction || lineAction);
      const printable = key.length === 1 && !event.ctrl && !event.meta;
      const browserDefaultSkipped = printable || key === "Tab"
        || (editable && (
          ["ArrowUp", "ArrowDown", "PageUp", "PageDown"].includes(key)
          || ((event.ctrl || event.alt || event.meta)
            && !selectAll
            && !wordAction
            && !lineAction
            && !["Control", "Alt", "Meta", "Shift"].includes(key))
        ));
      const unsupported = () => {
        throw Object.assign(new Error(`key ${JSON.stringify(key)} cannot be synthesized; use type for text`), { dispatched: true });
      };
      const keyEvent = new KeyboardEvent(event.phase === "down" ? "keydown" : "keyup", {
        key: event.key, code: event.code, shiftKey: event.shift, ctrlKey: event.ctrl,
        altKey: event.alt, metaKey: event.meta, bubbles: true, composed: true, cancelable: true,
      });
      target.dispatchEvent(keyEvent);
      if (event.phase !== "down" || keyEvent.defaultPrevented || document.activeElement !== target) return;
      if (selectAll) {
        if (input) {
          target.select();
        } else {
          const selection = window.getSelection();
          if (selection) {
            const range = document.createRange();
            range.selectNodeContents(target);
            selection.removeAllRanges();
            selection.addRange(range);
          }
        }
        return;
      }
      if (enter) {
        document.execCommand("insertLineBreak");
        return;
      }
      if (!editable) {
        if (browserDefaultSkipped) unsupported();
        return;
      }
      if (caretKeys.includes(key) && !caret) {
        if (browserDefaultSkipped) unsupported();
        return;
      }
      if (wordAction || lineAction) {
        const backward = key === "Backspace" || key === "ArrowLeft";
        const direction = backward ? "backward" : "forward";
        const granularity = wordAction ? "word" : "lineboundary";
        const moving = key === "ArrowLeft" || key === "ArrowRight";
        if (contenteditable) {
          const selection = window.getSelection();
          if (!selection) return;
          if (moving) {
            selection.modify(event.shift ? "extend" : "move", direction, granularity);
          } else {
            if (selection.isCollapsed) selection.modify("extend", direction, granularity);
            document.execCommand("delete");
          }
          return;
        }
        const selected = inputSelection(target);
        if (!selected) unsupported();
        const { start, end, backward: selectionBackward } = selected;
        const anchor = start === end ? start : selectionBackward ? end : start;
        const position = start === end ? start : selectionBackward ? start : end;
        const value = target.value;
        if (moving) {
          const next = !event.shift && start !== end
            ? backward ? start : end
            : lineAction
              ? lineBoundary(value, position, direction)
              : wordBoundary(value, position, direction);
          if (event.shift) {
            const selectionDirection = next < anchor ? "backward" : next > anchor ? "forward" : "none";
            setInputSelection(target, Math.min(anchor, next), Math.max(anchor, next), selectionDirection);
          } else {
            setInputSelection(target, next, next);
          }
        } else {
          if (start === end) {
            const boundary = lineAction
              ? lineBoundary(value, position, direction)
              : wordBoundary(value, position, direction);
            if (boundary === position) return;
            if (!setInputSelection(target, Math.min(boundary, position), Math.max(boundary, position))) {
              return;
            }
          }
          document.execCommand("delete");
        }
        return;
      }
      if (key === "Backspace") {
        document.execCommand("delete");
        return;
      }
      if (key === "Delete") {
        document.execCommand("forwardDelete");
        return;
      }
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(key)) {
        if (browserDefaultSkipped) unsupported();
        return;
      }
      if (input) {
        const value = target.value;
        const selected = inputSelection(target);
        if (!selected) unsupported();
        const { start, end, backward } = selected;
        const anchor = start === end ? start : backward ? end : start;
        let position = start === end ? start : backward ? start : end;
        if (!event.shift && start !== end && key === "ArrowLeft") {
          position = start;
        } else if (!event.shift && start !== end && key === "ArrowRight") {
          position = end;
        } else if (key === "ArrowLeft") {
          position = graphemeBoundary(value, position, "backward");
        } else if (key === "ArrowRight") {
          position = graphemeBoundary(value, position, "forward");
        } else if (key === "Home") {
          position = lineBoundary(value, position, "backward");
        } else {
          position = lineBoundary(value, position, "forward");
        }
        if (event.shift) {
          const direction = position < anchor ? "backward" : position > anchor ? "forward" : "none";
          setInputSelection(target, Math.min(anchor, position), Math.max(anchor, position), direction);
        } else {
          setInputSelection(target, position, position);
        }
      } else {
        const selection = window.getSelection();
        if (typeof selection?.modify === "function") {
          const direction = key === "ArrowLeft" || key === "Home" ? "backward" : "forward";
          const granularity = key === "Home" || key === "End" ? "lineboundary" : "character";
          selection.modify(event.shift ? "extend" : "move", direction, granularity);
        }
      }
      return;
    }
    if (event.event === "insert_text") {
      const target = document.activeElement;
      if (!target || !(target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target.isContentEditable)) {
        throw new Error("the focused element is not editable");
      }
      let owner;
      for (let el = target; el; el = el.parentElement) {
        if (el._id != null && this.views.get(el._id) === el) {
          owner = el;
          break;
        }
      }
      const subject = owner ? `node ${owner._id}` : "the focused input";
      if (target.disabled || owner?._props?.disabled) throw new Error(`${subject} is disabled`);
      if (target.readOnly) throw new Error(`${subject} is read-only`);
      if (document.execCommand("insertText", false, event.text)) return;
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
        const start = target.selectionStart ?? target.value.length;
        const end = target.selectionEnd ?? start;
        target.value = target.value.slice(0, start) + event.text + target.value.slice(end);
        target.setSelectionRange(start + event.text.length, start + event.text.length);
      } else {
        target.textContent = event.text;
      }
      target.dispatchEvent(new InputEvent("input", {
        data: event.text, inputType: "insertText", bubbles: true, composed: true,
      }));
      return;
    }
    if (event.event === "focus") {
      const element = this.views.get(event.node);
      const target = element?._field ?? element;
      if (!target?.isConnected) throw new Error(`no DOM element for node ${event.node}`);
      target.focus({ preventScroll: true });
      if (document.activeElement !== target) {
        throw new Error(`node ${event.node} cannot receive native focus`);
      }
      return;
    }
    if (event.event === "back") {
      history.back();
      return;
    }
    throw new Error(`unknown synthetic event ${JSON.stringify(event)}`);
  }
function question(query) {
switch (query.query) {
      case "input_text": {
        const field = this.views.get(query.id)?._field;
        // An agent's page shows a masked input as it is drawn: a bullet a
        // character, never the text.
        if (field && field.type === "password") return "\u2022".repeat(field.value.length);
        return field ? field.value : null;
      }
      case "hit_at": {
        // The browser's own hit testing: what `pointer-events: none` and
        // `inert` leave out it passes over, a canvas, image, frame or field
        // belongs to the view around it.
        for (let el = document.elementFromPoint(query.x, query.y); el; el = el.parentElement) {
          if (el._id != null && this.views.get(el._id) === el) return el._id;
        }
        return null;
      }
      case "focus": {
        const el = this.views.get(query.id);
        const target = el?._field ?? (el?.tabIndex >= 0 ? el : null);
        if (!target?.isConnected) return false;
        target.focus({ preventScroll: true });
        return document.activeElement === target;
      }
      case "focused": {
        for (let el = document.activeElement; el; el = el.parentElement) {
          if (el._id != null && this.views.get(el._id) === el) return el._id;
        }
        return null;
      }
      case "app_focused":
        return document.hasFocus();
}
throw new Error(`unknown page question ${query.query}`);
}
  function fromHash() {
    const parts = location.hash.slice(1).split("&");
    const supplied = parts.filter((part) => part.startsWith("gpui-agent="));
    if (supplied.length) {
      const rest = parts.filter((part) => !part.startsWith("gpui-agent="));
      history.replaceState(history.state, "", `${location.pathname}${location.search}${rest.length ? `#${rest.join("&")}` : ""}`);
      if (supplied.length !== 1) return null;
      const key = supplied[0].slice("gpui-agent=".length);
      if (!/^[0-9a-f]{32}$/.test(key)) return null;
      try {
        sessionStorage.setItem("nsh-agent-key", key);
      } catch {
        return key;
      }
      return key;
    }
    try {
      const key = sessionStorage.getItem("nsh-agent-key");
      return key && /^[0-9a-f]{32}$/.test(key) ? key : null;
    } catch {
      return null;
    }
  }
  let AGENT = globalThis.__nshAgentKey ?? fromHash();
  if (typeof AGENT !== "string" || !/^[0-9a-f]{32}$/.test(AGENT)) AGENT = null;
  delete globalThis.__nshAgentKey;

  if (AGENT) globalThis.nshHelloExtra = AGENT;

const handlers = globalThis.nshPageSteps ??= Object.create(null);
for (const name of ["hit_at", "focus", "focused", "app_focused", "input_text"]) {
handlers[name] = (args, player) => question.call(player, { ...args, query: name });
}
handlers.synth = (event, player) => {
  try {
    synthesize.call(player, event);
    return null;
  } catch (error) {
    if (error.dispatched) return { dispatched: true, error: error.message };
    throw error;
  }
};
})();
