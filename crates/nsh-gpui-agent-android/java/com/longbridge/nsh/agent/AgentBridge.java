package com.longbridge.nsh.agent;

import android.app.Activity;
import android.app.Dialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.ContextWrapper;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.graphics.RectF;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;
import android.util.SparseArray;
import android.view.ActionMode;
import android.view.InputDevice;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;

import java.io.File;
import java.io.IOException;
import java.lang.reflect.AccessibleObject;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.Arrays;
import java.util.concurrent.Callable;
import java.util.concurrent.FutureTask;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

/** The Android surface operations the agent adds, reached through reflection. */
public final class AgentBridge {
    static final int HIT_NONE = -1;
    static final int HIT_FOREIGN = -2;
    static final int HIT_FAILED = Integer.MIN_VALUE;

    private static final Handler MAIN = new Handler(Looper.getMainLooper());
    /**
     * The label of every clip this agent puts on the clipboard, with a token of this
     * process: a Paste the agent presses inserts only a clip this run set, never what a
     * person or another app copied (codes, passwords, addresses), which would then be read
     * back unmasked into proof.
     */
    static final String CLIP_LABEL = "nsh-agent:" + java.util.UUID.randomUUID();
    /**
     * The text of the clip this agent last put on the clipboard. The label can be seen by
     * whatever reads the clipboard (an input method, an app in front), which could reuse it
     * on a clip of its own: a clip is this run's only when its text is this one too.
     */
    private static volatile String lastClipText;

    /** Puts {@code text} on the clipboard as this run's clip. */
    private static void setAgentClip(ClipboardManager clipboard, CharSequence text) {
        lastClipText = text == null ? null : text.toString();
        clipboard.setPrimaryClip(ClipData.newPlainText(CLIP_LABEL, text));
    }
    private static String agentToken;

    private static volatile boolean reflectionReady;
    private static Class<?> boxClass;
    private static Class<?> inputBoxClass;
    private static Method nshMounter;
    private static Method mounterTopModal;
    private static Method inputShowKeyboard;
    private static Field mounterViews;
    private static Field boxPointerEvents;
    private static Field boxOverflow;
    private static Field boxClip;
    private static Field inputEdit;
    private static Field inputEditMenu;
    private static Field testAgentShownDialog;

    private AgentBridge() {}

    /** The agent's launch configuration: address, token and cache directory; null when off. */
    public static String[] agentConfig(View host) {
        try {
            Context context = host.getContext();
            Activity activity = activity(context);
            if (activity == null) return null;
            Intent intent = activity.getIntent();
            boolean agent = intent.getBooleanExtra("GPUI_AGENT", false)
                    || "1".equals(intent.getStringExtra("GPUI_AGENT"));
            if (!agent) return null;
            if ((context.getApplicationInfo().flags
                    & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) == 0) {
                return null;
            }
            String address = intent.getStringExtra("GPUI_AGENT_ADDR");
            String token = takeAgentToken(context);
            if (token != null) {
                agentToken = token;
            } else {
                token = agentToken;
            }
            if (token == null) {
                Log.w("nsh", "GPUI-agent not started: no token in files/gpui-agent-token");
                return null;
            }
            return new String[] {
                address != null ? address : "127.0.0.1:17421",
                token,
                context.getCacheDir().getAbsolutePath(),
            };
        } catch (Exception e) {
            Log.w("nsh", "GPUI-agent configuration failed: " + e);
            return null;
        }
    }

    /** Reads and deletes the token the command line wrote, without trimming it. */
    private static String takeAgentToken(Context context) {
        File file = new File(context.getFilesDir(), "gpui-agent-token");
        try {
            byte[] bytes = Files.readAllBytes(file.toPath());
            String token = new String(bytes, StandardCharsets.UTF_8);
            return token.isEmpty() ? null : token;
        } catch (IOException e) {
            return null;
        } finally {
            file.delete();
        }
    }

    /** The node whose view takes a touch at ({@code x}, {@code y}). */
    public static int hitAt(View host, float x, float y) {
        return onMain(() -> {
            View hit = topmost(target(host), x, y);
            SparseArray views = views(host);
            for (View view = hit; view != null; view = view.getParent() instanceof View
                    ? (View) view.getParent() : null) {
                for (int i = 0; i < views.size(); i++) {
                    if (views.valueAt(i) == view) return views.keyAt(i);
                }
            }
            return hit == null ? HIT_NONE : HIT_FOREIGN;
        }, HIT_FAILED);
    }

    /** The focused input's node id, or {@link #HIT_NONE}. */
    public static int focusedNode(View host) {
        return onMain(() -> {
            View focus = focused(host);
            SparseArray views = views(host);
            for (View view = focus; view != null; view = view.getParent() instanceof View
                    ? (View) view.getParent() : null) {
                int index = views.indexOfValue(view);
                if (index >= 0) {
                    return inputBoxClass.isInstance(view) ? views.keyAt(index) : HIT_NONE;
                }
            }
            return HIT_NONE;
        }, HIT_FAILED);
    }

    /** The surface's own size: what {@link #resizeViewport} sets. */
    public static float[] contentSize(View host) {
        return onMain(() -> new float[] {host.getWidth(), host.getHeight()});
    }

    /** Focuses input node {@code id}. */
    public static boolean focusNode(View host, int id) {
        return onMain(() -> {
            Object view = views(host).get(id);
            if (!inputBoxClass.isInstance(view)) return false;
            host.getRootView().clearFocus();
            View edit = (View) inputEdit.get(view);
            edit.setFocusableInTouchMode(true);
            return edit.requestFocus();
        }, false);
    }

    /**
     * The text menu Android shows over an input now, read-only: its kind, the node id
     * of the field it is over (empty when it cannot be told), then a fixed
     * word for each visible item in order ({@link #itemWord}: never the item's title,
     * which an OEM or a PROCESS_TEXT app may fill with what it likes, and the snapshot
     * lands in proof folders), and only the kind for a masked field. {@code "selection"} and {@code "insertion"}
     * are the field's own floating toolbar (EditText's, over a selection or at the
     * caret: Android draws the matching handles with it); {@code "edit"} is the one
     * the kit opens over a selection it draws itself (InputBox.openEditMenu). Empty
     * when none shows. Each call first watches every input it has not watched yet
     * ({@link MenuWatch}), so a menu opened after the first snapshot is seen.
     */
    public static String[] textMenu(View host) {
        return onMain(() -> {
            SparseArray views = views(host);
            java.lang.ref.WeakReference<ActionMode> showing = MenuWatch.shown;
            ActionMode shown = showing == null ? null : showing.get();
            java.lang.ref.WeakReference<EditText> owning = MenuWatch.field;
            EditText owner = owning == null ? null : owning.get();
            String[] edit = null;
            int ownerId = HIT_NONE;
            for (int i = 0; i < views.size(); i++) {
                Object view = views.valueAt(i);
                if (!inputBoxClass.isInstance(view)) continue;
                try {
                    EditText field = (EditText) inputEdit.get(view);
                    if (field != null) MenuWatch.watch(field);
                    if (field != null && field == owner) ownerId = views.keyAt(i);
                    ActionMode own = (ActionMode) inputEditMenu.get(view);
                    if (own != null && edit == null) {
                        edit = menuOf("edit", views.keyAt(i), own.getMenu(), masked(field));
                    }
                } catch (Exception e) {
                    Log.w("nsh", "test agent: text menu of " + views.keyAt(i) + ": " + e);
                }
            }
            if (shown != null) return menuOf(MenuWatch.kind, ownerId, shown.getMenu(), masked(owner));
            return edit != null ? edit : new String[0];
        }, new String[0]);
    }

    /**
     * Presses item {@code word} of the text menu showing now (the one {@link #textMenu}
     * reports), through that menu: {@code performIdentifierAction} runs the item as the
     * toolbar does when a finger taps it, so the field's own handler (Cut, Copy, Paste,
     * Select all) acts. Only those four: an item that leaves the app is never pressed.
     * Returns an empty string when pressed, else why not: no menu, the item absent or
     * disabled, or a masked field's text that Cut or Copy would give away.
     */
    public static String pressTextMenuItem(View host, String word) {
        String refused = onMain(() -> {
            int id = itemId(word);
            if (id == 0) return "`" + word + "` is not pressed";
            SparseArray views = views(host);
            java.lang.ref.WeakReference<ActionMode> showing = MenuWatch.shown;
            ActionMode menu = showing == null ? null : showing.get();
            java.lang.ref.WeakReference<EditText> owning = MenuWatch.field;
            EditText field = owning == null ? null : owning.get();
            if (menu == null) {
                // The kit's own edit menu, as textMenu falls back to it.
                for (int i = 0; i < views.size() && menu == null; i++) {
                    Object view = views.valueAt(i);
                    if (!inputBoxClass.isInstance(view)) continue;
                    ActionMode own = (ActionMode) inputEditMenu.get(view);
                    if (own != null) {
                        menu = own;
                        field = (EditText) inputEdit.get(view);
                    }
                }
            }
            if (menu == null) return "no text menu shows";
            if ((id == android.R.id.cut || id == android.R.id.copy) && masked(field)) {
                return "the field is masked: `" + word + "` would give its text away";
            }
            MenuItem item = menu.getMenu().findItem(id);
            if (item == null || !item.isVisible()) return "the text menu has no `" + word + "`";
            if (!item.isEnabled()) return "`" + word + "` is disabled in the text menu";
            ClipboardManager clipboard = (ClipboardManager)
                    host.getContext().getSystemService(Context.CLIPBOARD_SERVICE);
            if (id == android.R.id.paste && !agentClip(clipboard)) {
                return "the clipboard was not set by this run: Paste would put in what a person or "
                        + "another app copied";
            }
            if (!menu.getMenu().performIdentifierAction(id, 0)) {
                return "the text menu did not take `" + word + "`";
            }
            // What Cut or Copy just put there is this run's clip: labelled so, a
            // Paste after it is allowed.
            if ((id == android.R.id.cut || id == android.R.id.copy) && clipboard != null) {
                ClipData clip = clipboard.getPrimaryClip();
                if (clip != null && clip.getItemCount() > 0) {
                    CharSequence text = clip.getItemAt(0).coerceToText(host.getContext());
                    setAgentClip(clipboard, text);
                }
            }
            return "";
        });
        return refused == null ? mainFailure("press of `" + word + "`") : refused;
    }

    /** Whether the clipboard's clip is one this run put there ({@link #CLIP_LABEL}). */
    private static boolean agentClip(ClipboardManager clipboard) {
        if (clipboard == null || !clipboard.hasPrimaryClip()) return false;
        android.content.ClipDescription description = clipboard.getPrimaryClipDescription();
        CharSequence label = description == null ? null : description.getLabel();
        if (label == null || !CLIP_LABEL.contentEquals(label)) return false;
        // The label alone could be another's copy of it: the text must be the one set.
        String expected = lastClipText;
        ClipData clip = clipboard.getPrimaryClip();
        if (expected == null || clip == null || clip.getItemCount() != 1) return false;
        CharSequence text = clip.getItemAt(0).getText();
        return text != null && expected.contentEquals(text);
    }

    /** {@code 1} when the clipboard holds a clip this run set, {@code 0} when it holds none or another's, {@code -1} when it cannot be told. */
    public static int clipboardSetByAgent(View host) {
        Integer owned = onMain(() -> {
            ClipboardManager clipboard = (ClipboardManager)
                    host.getContext().getSystemService(Context.CLIPBOARD_SERVICE);
            return agentClip(clipboard) ? 1 : 0;
        });
        return owned == null ? -1 : owned;
    }

    /** The android.R.id of a pressable menu word; 0 for any other word. */
    private static int itemId(String word) {
        switch (word) {
            case "cut": return android.R.id.cut;
            case "copy": return android.R.id.copy;
            case "paste": return android.R.id.paste;
            case "select-all": return android.R.id.selectAll;
            default: return 0;
        }
    }

    /**
     * {@code kind}, the node id of the field it is over ({@code field}, or an empty
     * string when it is not known), and a word for each of {@code menu}'s visible
     * items, in order; no items when {@code masked}.
     */
    private static String[] menuOf(String kind, int field, Menu menu, boolean masked) {
        java.util.ArrayList<String> out = new java.util.ArrayList<>();
        out.add(kind);
        out.add(field >= 0 ? Integer.toString(field) : "");
        for (int i = 0; !masked && menu != null && i < menu.size(); i++) {
            MenuItem item = menu.getItem(i);
            if (item.isVisible()) out.add(itemWord(item));
        }
        return out.toArray(new String[0]);
    }

    /** A fixed word for a text menu item by its id; {@code "other"} for any item Android does not define. */
    private static String itemWord(MenuItem item) {
        int id = item.getItemId();
        if (id == android.R.id.cut) return "cut";
        if (id == android.R.id.copy) return "copy";
        if (id == android.R.id.paste) return "paste";
        if (id == android.R.id.pasteAsPlainText) return "paste-as-plain-text";
        if (id == android.R.id.selectAll) return "select-all";
        if (id == android.R.id.replaceText) return "replace";
        if (id == android.R.id.shareText) return "share";
        if (id == android.R.id.textAssist) return "assist";
        if (id == android.R.id.autofill) return "autofill";
        return "other";
    }

    /** Whether {@code field} hides its text (a password or PIN field); a field that is gone counts as masked. */
    private static boolean masked(EditText field) {
        if (field == null) return true;
        if (field.getTransformationMethod() instanceof android.text.method.PasswordTransformationMethod) return true;
        int type = field.getInputType();
        int variation = type & android.text.InputType.TYPE_MASK_VARIATION;
        int kind = type & android.text.InputType.TYPE_MASK_CLASS;
        return (kind == android.text.InputType.TYPE_CLASS_TEXT
                        && (variation == android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
                                || variation == android.text.InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                                || variation == android.text.InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD))
                || (kind == android.text.InputType.TYPE_CLASS_NUMBER
                        && variation == android.text.InputType.TYPE_NUMBER_VARIATION_PASSWORD);
    }

    /**
     * Watches an EditText's own floating toolbar by wrapping its custom selection and
     * insertion callbacks: every call goes on to the callback that was there (none,
     * in NSH's fields), and with none it answers as Android does without one (keep the
     * menu, leave the items to the field), so the field behaves exactly as before. It
     * only records which toolbar shows.
     */
    private static final class MenuWatch implements ActionMode.Callback {
        /**
         * The toolbar showing now, its kind, and the field it is over: weakly, so a
         * toolbar whose destroy never comes cannot keep its window's views alive.
         */
        static volatile java.lang.ref.WeakReference<ActionMode> shown;
        static volatile String kind;
        static volatile java.lang.ref.WeakReference<EditText> field;

        private final ActionMode.Callback inner;
        private final String watched;
        private final java.lang.ref.WeakReference<EditText> owner;

        private MenuWatch(ActionMode.Callback inner, String watched, EditText owner) {
            this.inner = inner;
            this.watched = watched;
            this.owner = new java.lang.ref.WeakReference<>(owner);
        }

        static void watch(EditText field) {
            ActionMode.Callback selection = field.getCustomSelectionActionModeCallback();
            if (!(selection instanceof MenuWatch)) {
                field.setCustomSelectionActionModeCallback(new MenuWatch(selection, "selection", field));
            }
            if (Build.VERSION.SDK_INT >= 23) {
                ActionMode.Callback insertion = field.getCustomInsertionActionModeCallback();
                if (!(insertion instanceof MenuWatch)) {
                    field.setCustomInsertionActionModeCallback(new MenuWatch(insertion, "insertion", field));
                }
            }
        }

        @Override
        public boolean onCreateActionMode(ActionMode mode, Menu menu) {
            boolean keep = inner == null || inner.onCreateActionMode(mode, menu);
            if (keep) {
                shown = new java.lang.ref.WeakReference<>(mode);
                kind = watched;
                field = owner;
            }
            return keep;
        }

        @Override
        public boolean onPrepareActionMode(ActionMode mode, Menu menu) {
            return inner == null || inner.onPrepareActionMode(mode, menu);
        }

        @Override
        public boolean onActionItemClicked(ActionMode mode, MenuItem item) {
            return inner != null && inner.onActionItemClicked(mode, item);
        }

        @Override
        public void onDestroyActionMode(ActionMode mode) {
            try {
                if (inner != null) inner.onDestroyActionMode(mode);
            } finally {
                // Cleared even when the callback it wraps throws.
                java.lang.ref.WeakReference<ActionMode> showing = shown;
                if (showing != null && showing.get() == mode) {
                    shown = null;
                    field = null;
                }
            }
        }
    }

    /**
     * The selections of input nodes {@code ids} as their EditTexts hold them,
     * in one main-thread hop: an (id, start, end) triple for each that has
     * one, in UTF-16 code units, start first (the direction a selection was
     * made in is dropped; a bare caret has equal ends). Ids that are no
     * input, or whose field has no selection, are left out.
     */
    public static int[] inputSelections(View host, int[] ids) {
        return onMain(() -> {
            SparseArray views = views(host);
            int[] out = new int[ids.length * 3];
            int n = 0;
            for (int id : ids) {
                // One field that cannot be read leaves the others' selections.
                try {
                    Object view = views.get(id);
                    if (!inputBoxClass.isInstance(view)) continue;
                    EditText edit = (EditText) inputEdit.get(view);
                    if (edit == null) continue;
                    int start = edit.getSelectionStart(), end = edit.getSelectionEnd();
                    if (start < 0 || end < 0) continue;
                    out[n++] = id;
                    out[n++] = Math.min(start, end);
                    out[n++] = Math.max(start, end);
                } catch (Exception e) {
                    Log.w("nsh", "test agent: selection of " + id + ": " + e);
                }
            }
            return java.util.Arrays.copyOf(out, n);
        }, new int[0]);
    }

    /** Whether the surface's window has focus. */
    public static boolean appFocused(View host) {
        return onMain(host::hasWindowFocus, false);
    }

    /** Resizes the surface within the window. */
    public static boolean resizeViewport(View host, int width, int height) {
        return onMain(() -> {
            View root = host.getRootView();
            if (width <= 0 || height <= 0 || width > root.getWidth() || height > root.getHeight()) {
                return false;
            }
            ViewGroup.LayoutParams params = host.getLayoutParams();
            if (params == null) return false;
            params.width = width;
            params.height = height;
            host.setLayoutParams(params);
            host.requestLayout();
            root.requestLayout();
            return true;
        }, false);
    }

    /** Moves a mouse over the surface without a button. */
    public static boolean hover(View host, float x, float y) {
        return onMain(() -> {
            View root = target(host);
            long now = SystemClock.uptimeMillis();
            MotionEvent event = MotionEvent.obtain(now, now, MotionEvent.ACTION_HOVER_ENTER, x, y, 0);
            event.setSource(InputDevice.SOURCE_MOUSE);
            root.dispatchGenericMotionEvent(event);
            event.recycle();
            event = MotionEvent.obtain(now, now, MotionEvent.ACTION_HOVER_MOVE, x, y, 0);
            event.setSource(InputDevice.SOURCE_MOUSE);
            root.dispatchGenericMotionEvent(event);
            event.recycle();
            return true;
        }, false);
    }

    /** Requests the device orientation. */
    public static boolean rotate(View host, boolean landscape) {
        Boolean done = onMain(() -> {
            Activity activity = activity(host.getContext());
            if (activity == null || activity.isFinishing()) return false;
            activity.setRequestedOrientation(landscape
                    ? ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
                    : ActivityInfo.SCREEN_ORIENTATION_PORTRAIT);
            return true;
        });
        return done != null && done;
    }

    /** Shows the keyboard for the focused input, or hides it from this window. */
    public static boolean keyboard(View host, boolean shown) {
        Boolean done = onMain(() -> {
            if (shown) {
                View focus = focused(host);
                Object parent = focus == null ? null : focus.getParent();
                if (!(focus instanceof EditText) || !inputBoxClass.isInstance(parent)) return false;
                inputShowKeyboard.invoke(parent);
                return true;
            }
            View focus = focused(host);
            InputMethodManager manager = (InputMethodManager)
                    host.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (manager == null) return false;
            manager.hideSoftInputFromWindow(
                    focus == null ? host.getWindowToken() : focus.getWindowToken(), 0);
            return true;
        });
        return done != null && done;
    }

    /** Whether the input method is visible: 1, 0, or -1 when unknown. */
    public static int imeState(View host) {
        return onMain(() -> {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return -1;
            View view = focused(host);
            if (view == null) view = host;
            WindowInsets insets = view.getRootWindowInsets();
            if (insets == null) return -1;
            return insets.isVisible(WindowInsets.Type.ime()) ? 1 : 0;
        }, -1);
    }

    /**
     * Where the input method covers the window touches go to (a dialog's or
     * a modal's own window over the app, else the app's: {@link #target}),
     * in its pixels: x, y, width, height. An empty array while it is hidden
     * or the window is not attached yet; null when this Android cannot tell
     * (before API 30). A landscape fullscreen ("extract") input method
     * reports no bottom inset and counts as hidden.
     */
    public static float[] keyboardFrame(View host) {
        return onMain(() -> {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null;
            View root = target(host).getRootView();
            WindowInsets insets = root.getRootWindowInsets();
            if (insets == null || !insets.isVisible(WindowInsets.Type.ime())) return new float[0];
            int bottom = insets.getInsets(WindowInsets.Type.ime()).bottom;
            if (bottom <= 0) return new float[0];
            return new float[] {0, root.getHeight() - bottom, root.getWidth(), bottom};
        }, null);
    }

    /** Pastes plain text into the focused input and restores the clipboard. */
    public static boolean paste(View host, String text) {
        Boolean done = onMain(() -> {
            View focus = focused(host);
            Object parent = focus == null ? null : focus.getParent();
            if (!(focus instanceof EditText) || !inputBoxClass.isInstance(parent)) return false;
            ClipboardManager clipboard = (ClipboardManager)
                    host.getContext().getSystemService(Context.CLIPBOARD_SERVICE);
            if (clipboard == null) return false;
            ClipData previous = clipboard.getPrimaryClip();
            try {
                setAgentClip(clipboard, text);
                return ((EditText) focus).onTextContextMenuItem(android.R.id.paste);
            } finally {
                if (previous == null && Build.VERSION.SDK_INT >= 28) clipboard.clearPrimaryClip();
                else if (previous == null) clipboard.setPrimaryClip(ClipData.newPlainText("", ""));
                else clipboard.setPrimaryClip(previous);
            }
        });
        return done != null && done;
    }

    /**
     * The innermost view that takes a touch at ({@code x}, {@code y}) of
     * {@code v}: the highest Z first and, of equal Z, the last one first.
     */
    private static View topmost(View v, float x, float y) throws Exception {
        initReflection(v);
        boolean box = boxClass.isInstance(v);
        int pointerEvents = box ? boxPointerEvents.getInt(v) : 0;
        if (pointerEvents == 1 || v.getVisibility() != View.VISIBLE) return null;
        if (v instanceof ViewGroup && pointerEvents != 3) {
            ViewGroup g = (ViewGroup) v;
            View[] children = new View[g.getChildCount()];
            for (int i = 0; i < children.length; i++) children[i] = g.getChildAt(i);
            // Stable: of equal Z, the children stay in their order.
            Arrays.sort(children, (a, b) -> Float.compare(a.getZ(), b.getZ()));
            float[] pt = new float[2];
            for (int i = children.length - 1; i >= 0; i--) {
                View c = children[i];
                pt[0] = x + g.getScrollX() - c.getLeft();
                pt[1] = y + g.getScrollY() - c.getTop();
                android.graphics.Matrix m = c.getMatrix();
                if (!m.isIdentity()) {
                    android.graphics.Matrix inverse = new android.graphics.Matrix();
                    if (m.invert(inverse)) inverse.mapPoints(pt);
                }
                boolean inside = pt[0] >= 0 && pt[1] >= 0 && pt[0] < c.getWidth() && pt[1] < c.getHeight();
                RectF overflow = boxClass.isInstance(c) ? (RectF) boxOverflow.get(c) : null;
                boolean clip = !boxClass.isInstance(c) || boxClip.getBoolean(c);
                if (!inside && !(overflow != null && !clip && overflow.contains(pt[0], pt[1]))) continue;
                View hit = topmost(c, pt[0], pt[1]);
                if (hit != null) return hit;
            }
        }
        if (pointerEvents == 2) return null;
        // Reached through its overflow area: only its descendants count.
        if (x < 0 || y < 0 || x >= v.getWidth() || y >= v.getHeight()) return null;
        return v;
    }

    /** Where touches go: a dialog over the app, else the app's window. */
    private static View target(View host) throws Exception {
        initReflection(host);
        Dialog d = (Dialog) testAgentShownDialog.get(null);
        if (d != null && d.isShowing() && d.getWindow() != null) return d.getWindow().getDecorView();
        View modal = (View) mounterTopModal.invoke(mounter(host));
        return modal != null ? modal : host.getRootView();
    }

    /** The focused view keys go to. */
    private static View focused(View host) throws Exception {
        View focus = target(host).findFocus();
        return focus != null ? focus : host.getRootView().findFocus();
    }

    private static Object mounter(View host) throws Exception {
        initReflection(host);
        return nshMounter.invoke(host);
    }

    @SuppressWarnings("rawtypes")
    private static SparseArray views(View host) throws Exception {
        return (SparseArray) mounterViews.get(mounter(host));
    }

    private static Activity activity(Context context) {
        while (context instanceof ContextWrapper) {
            if (context instanceof Activity) return (Activity) context;
            context = ((ContextWrapper) context).getBaseContext();
        }
        return null;
    }

    /** Reflective access to the host's package-private internals. */
    private static void initReflection(View host) throws Exception {
        if (reflectionReady) return;
        synchronized (AgentBridge.class) {
            if (reflectionReady) return;
            ClassLoader loader = host.getClass().getClassLoader();
            Class<?> nshView = Class.forName("com.longbridge.nsh.NshView", false, loader);
            Class<?> mounter = Class.forName("com.longbridge.nsh.Mounter", false, loader);
            boxClass = Class.forName("com.longbridge.nsh.Box", false, loader);
            inputBoxClass = Class.forName("com.longbridge.nsh.InputBox", false, loader);
            Class<?> testAgent = Class.forName("com.longbridge.nsh.TestAgent", false, loader);

            nshMounter = nshView.getDeclaredMethod("mounter");
            mounterTopModal = mounter.getDeclaredMethod("topModalWindow");
            inputShowKeyboard = inputBoxClass.getDeclaredMethod("showKeyboard");
            mounterViews = mounter.getDeclaredField("views");
            boxPointerEvents = boxClass.getDeclaredField("pointerEvents");
            boxOverflow = boxClass.getDeclaredField("overflow");
            boxClip = boxClass.getDeclaredField("clip");
            inputEdit = inputBoxClass.getDeclaredField("edit");
            inputEditMenu = inputBoxClass.getDeclaredField("editMenu");
            testAgentShownDialog = testAgent.getDeclaredField("shownDialog");
            for (AccessibleObject object : new AccessibleObject[] {
                nshMounter, mounterTopModal, inputShowKeyboard, mounterViews, boxPointerEvents,
                boxOverflow, boxClip, inputEdit, inputEditMenu, testAgentShownDialog,
            }) {
                object.setAccessible(true);
            }
            reflectionReady = true;
        }
    }

    /** {@code call} on the main thread, waited for (10 s at most). */
    private static <T> T onMain(Callable<T> call) {
        MAIN_STATE.get()[0] = MAIN_ANSWERED;
        // Already on the main thread: posting and waiting would wait on
        // ourselves until the timeout.
        if (Looper.myLooper() == Looper.getMainLooper()) {
            try {
                return call.call();
            } catch (Exception e) {
                Log.w("nsh", "test agent: " + e);
                return null;
            }
        }
        FutureTask<T> task = new FutureTask<>(call);
        MAIN.post(task);
        try {
            return task.get(10, TimeUnit.SECONDS);
        } catch (TimeoutException e) {
            // Never left queued to run after the caller was told it failed (a
            // retried paste would paste twice): withdrawn while it has not
            // started, else waited for, since a started call runs to its end.
            if (task.cancel(false)) {
                MAIN.removeCallbacks(task);
                MAIN_STATE.get()[0] = MAIN_WITHDRAWN;
                Log.w("nsh", "test agent: the main thread did not take a call in 10 s; withdrawn");
                return null;
            }
            try {
                return task.get(10, TimeUnit.SECONDS);
            } catch (Exception again) {
                MAIN_STATE.get()[0] = MAIN_UNCERTAIN;
                Log.w("nsh", "test agent: a main-thread call is still running after 20 s: " + again);
                return null;
            }
        } catch (Exception e) {
            Log.w("nsh", "test agent: " + e);
            return null;
        }
    }

    /** How the calling thread's last {@link #onMain} call ended: answered, withdrawn before it ran, or still running. */
    private static final int MAIN_ANSWERED = 0, MAIN_WITHDRAWN = 1, MAIN_UNCERTAIN = 2;
    private static final ThreadLocal<int[]> MAIN_STATE = new ThreadLocal<int[]>() {
        @Override
        protected int[] initialValue() {
            return new int[1];
        }
    };

    /**
     * How the calling thread's last main-thread call ended ({@code 0} answered, {@code 1}
     * withdrawn before it ran, {@code 2} may still be running), and forgets it: the driver
     * asks after a call that acts and failed, to say whether it happened.
     */
    public static int takeMainState() {
        int[] state = MAIN_STATE.get();
        int last = state[0];
        state[0] = MAIN_ANSWERED;
        return last;
    }

    /** Why an acting call that answered nothing failed, for the driver's reply. */
    private static String mainFailure(String action) {
        switch (takeMainState()) {
            case MAIN_WITHDRAWN:
                return "mailbox_full: the Android main thread did not take the " + action
                        + " within 10 s; it was withdrawn and does not run";
            case MAIN_UNCERTAIN:
                return "the Android main thread did not finish the " + action
                        + " in time: it may still happen (delivery uncertain)";
            default:
                return "the " + action + " failed on the Android main thread";
        }
    }

    /** {@code call} on the main thread; {@code fallback} on failure. */
    private static <T> T onMain(Callable<T> call, T fallback) {
        T value = onMain(call);
        return value == null ? fallback : value;
    }
}
