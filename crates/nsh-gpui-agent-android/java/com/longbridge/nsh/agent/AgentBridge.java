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

/** The Android surface operations the agent adds, reached through reflection. */
public final class AgentBridge {
    static final int HIT_NONE = -1;
    static final int HIT_FOREIGN = -2;
    static final int HIT_FAILED = Integer.MIN_VALUE;

    private static final Handler MAIN = new Handler(Looper.getMainLooper());
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
     * The text menu Android shows over an input now, read-only: its kind, then a fixed
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
            String[] edit = null;
            for (int i = 0; i < views.size(); i++) {
                Object view = views.valueAt(i);
                if (!inputBoxClass.isInstance(view)) continue;
                try {
                    EditText field = (EditText) inputEdit.get(view);
                    if (field != null) MenuWatch.watch(field);
                    ActionMode own = (ActionMode) inputEditMenu.get(view);
                    if (own != null && edit == null) edit = menuOf("edit", own.getMenu(), masked(field));
                } catch (Exception e) {
                    Log.w("nsh", "test agent: text menu of " + views.keyAt(i) + ": " + e);
                }
            }
            ActionMode shown = MenuWatch.shown;
            if (shown != null) {
                java.lang.ref.WeakReference<EditText> owner = MenuWatch.field;
                return menuOf(MenuWatch.kind, shown.getMenu(), masked(owner == null ? null : owner.get()));
            }
            return edit != null ? edit : new String[0];
        }, new String[0]);
    }

    /** {@code kind} and a word for each of {@code menu}'s visible items, in order; only the kind when {@code masked}. */
    private static String[] menuOf(String kind, Menu menu, boolean masked) {
        java.util.ArrayList<String> out = new java.util.ArrayList<>();
        out.add(kind);
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
        /** The toolbar showing now, its kind, and the field it is over (weakly). */
        static volatile ActionMode shown;
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
                shown = mode;
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
            if (inner != null) inner.onDestroyActionMode(mode);
            if (shown == mode) shown = null;
        }
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
                clipboard.setPrimaryClip(ClipData.newPlainText("text", text));
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
        FutureTask<T> task = new FutureTask<>(call);
        MAIN.post(task);
        try {
            return task.get(10, TimeUnit.SECONDS);
        } catch (Exception e) {
            Log.w("nsh", "test agent: " + e);
            return null;
        }
    }

    /** {@code call} on the main thread; {@code fallback} on failure. */
    private static <T> T onMain(Callable<T> call, T fallback) {
        T value = onMain(call);
        return value == null ? fallback : value;
    }
}
