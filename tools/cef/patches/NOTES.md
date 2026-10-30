# CEF patches P1 and P2 (draft)

Drafted 2026-10-09 against CEF `708dc140c`, Chromium 152.0.7977.83, branch 7977. Nothing here was compiled. Every patch passes `git apply --check`; see "Apply checks". The spec is `docs/superpowers/specs/2026-10-09-browser-extensions-cef-fork-design.md`.

## Files

| File | Applies in | What it is |
|---|---|---|
| `p1-tab-window-model.patch` | `chromium/src`, with `-p0` | Chromium side of P1. Register in `patch.cfg`. |
| `p1-cef-api.patch` | `chromium/src/cef`, with `-p0` | CEF side of P1. Normal source edits plus 4 new files. |
| `p2-no-native-windows.patch` | `chromium/src`, with `-p0` | Chromium side of P2. Register in `patch.cfg`. |
| `p2-cef-api.patch` | `chromium/src/cef`, with `-p0` | CEF side of P2. Normal source edits plus 2 new files. |

The paths have no `a/` and `b/` prefix, the same as the files in `cef/patch/patches/`. Use `git apply -p0`.

Order: P1 before P2. P2 needs `alloy_extension_tabs.h` from P1 (`GetAlloyActiveTab`). The two Chromium patches do not touch the same lines, and the two CEF patches do not either. All four were applied in order to a scratch copy of the original files with no conflict.

## P1: tab and window model

Goal: an Alloy browser that the app registers shows up in `chrome.tabs.query`, `chrome.windows.getAll/get/getCurrent/getLastFocused` and the tab events.

### New public API (`include/cef_browser.h`, class `CefBrowserHost`)

```cpp
#if CEF_API_ADDED(CEF_EXPERIMENTAL)
  /*--cef(added=experimental)--*/
  virtual void SetExtensionTabInfo(int window_id, bool active) = 0;

  /*--cef(added=experimental)--*/
  static void SetExtensionFocusedWindow(int window_id);
#endif
```

Rules:

- `window_id` must be greater than 0. Chromium's `SessionID` treats 0 and below as invalid. A value of 0 or less removes the browser from the tab model.
- The window id shares a number space with Chromium's own `SessionID` values (tab ids, Browser windows). Alloy has no Browser windows, but tab ids are small integers from 1 up. The app should use large window ids, for example from 16777216 up, so they never equal a tab id.
- Browsers with the same `window_id` are the tabs of one window. Tab index is the order of the first call. There is no way to reorder yet.
- `active = true` clears the flag on the other tabs of that window.
- A browser the app never registers is not in `tabs.query`. It is still found by `tabs.get(id)`, as before. Register content tabs only. Do not register extension popups, DevTools or helper surfaces.
- `SetExtensionFocusedWindow(id <= 0)` means no window has focus. `getLastFocused` and `lastFocusedWindow: true` still answer with the window that had focus last.
- Both methods are safe from any thread. They post to the UI thread. Chrome style browsers ignore `SetExtensionTabInfo`.
- Events fired: `tabs.onCreated` (first registration), `tabs.onActivated` (tab becomes active), `tabs.onUpdated` (status, url, title), `tabs.onRemoved` (browser destroyed or unregistered), `windows.onCreated`, `windows.onRemoved` (first and last tab of a window), `windows.onFocusChanged`.

### How it works

CEF side (`p1-cef-api.patch`):

- `libcef/browser/chrome/extensions/alloy_extension_tabs.{h,cc}`: a registry on the UI thread. One `TabEntry` (a `WebContentsObserver`) per registered `WebContents`, so entries remove themselves when the contents dies. Sets the window id on the existing `SessionTabHelper`, so `tab.windowId` is right everywhere. Fires the events through new wrappers on `TabsEventRouter`.
- `libcef/browser/chrome/extensions/alloy_window_controller.{h,cc}`: an `extensions::WindowController` per window id. It registers itself in `WindowControllerList`, so `windows.get`, `windows.getCurrent`, `tabs.getAllInWindow` and `ExtensionTabUtil::GetControllerFromWindowID` work with no change. It has no `ui::BaseWindow` (`window()` is null). `left` and `top` are 0, `width` and `height` come from the active tab.
- `browser_host_base.{h,cc}`: `CefBrowserHostBase::SetExtensionTabInfo`.
- `BUILD.gn`: the 4 new files.
- The static `CefBrowserHost::SetExtensionFocusedWindow` is defined at the end of `alloy_extension_tabs.cc`.

Chromium side (`p1-tab-window-model.patch`), all code under `BUILDFLAG(ENABLE_CEF)` except the two small null-safety changes:

| File | Change |
|---|---|
| `extensions/api/tabs/tabs_api.cc` | `TabsQueryFunction::BuildTabList` calls new `AppendAlloyTabs`, with a new `MatchesAlloyTab`. They apply `windowId`, `windowType`, `currentWindow`, `lastFocusedWindow`, `index`, `active`, `highlighted`, `url`, `title`, `status` and the rest. `WindowsGetAll`, `WindowsGetLastFocused` and `TabsGetSelected` get an Alloy branch. `WindowsRemove` returns an error when the window has no `ui::BaseWindow` (not under ifdef, it prevents a null crash). |
| `extensions/api/tabs/tabs_api.h` | Declares `AppendAlloyTabs` and `MatchesAlloyTab`. |
| `extensions/extension_tab_util.cc` | `CreateTabObject`: for a tab with no `TabInterface`, calls `cef::UpdateAlloyTabObject` to set `index`, `windowId`, `active`, `selected`, `highlighted`. |
| `extensions/chrome_extension_function_details.cc` | `GetCurrentWindowController` returns the Alloy current window before it looks for Browser windows. |
| `extensions/window_controller.{h,cc}` | New virtual `IsActive()`. The default is `window() && window()->IsActive()`. |
| `extensions/window_controller_list.cc` | `CurrentWindowForFunctionWithFilter` uses `IsActive()` and skips `CalledFromChildWindow` when `window()` is null. |
| `extensions/api/tabs/tabs_event_router.{h,cc}` | Public wrappers `DispatchAlloyTabCreated/Updated/Activated/Removed`. |
| `extensions/api/tabs/windows_event_router.cc` | `windows.onCreated` and `onRemoved` no longer skip a controller that has no `BrowserWindowInterface` if it is an Alloy one. |

"Current window" for a call: the window of the tab that sent the call, else the focused window, else the last focused one, else the oldest window of the profile.

### Register in `cef/patch/patch.cfg`

Put this stanza directly after the `chrome_browser_extensions` entry (line 362 in this checkout). It must come after, because it patches lines that patch already changed in `extension_tab_util.cc` (the `#if BUILDFLAG(ENABLE_CEF)` include block).

```python
  {
    # chrome: Expose Alloy style browsers that the client registered with
    # CefBrowserHost::SetExtensionTabInfo to chrome.tabs and chrome.windows.
    # Our own fork, no upstream issue.
    'name': 'p1-tab-window-model',
  },
```

Copy `p1-tab-window-model.patch` to `cef/patch/patches/p1-tab-window-model.patch`.

The CEF-side files are not in `patch.cfg`. Apply them in the cef checkout and commit them there:

```
cd chromium/src/cef && git apply -p0 /path/to/p1-cef-api.patch
```

## P2: no native windows

Goal: `chrome.tabs.create` and `chrome.windows.create` never open a native window when the profile has an Alloy browser. The app decides what to open.

### New public API (`include/cef_life_span_handler.h`, class `CefLifeSpanHandler`)

```cpp
#if CEF_API_ADDED(CEF_EXPERIMENTAL)
  /*--cef(added=experimental,optional_param=url)--*/
  virtual bool OnExtensionCreateTab(CefRefPtr<CefBrowser> browser,
                                    const CefString& extension_id,
                                    const CefString& url,
                                    int window_id,
                                    bool active,
                                    bool new_window,
                                    CefRefPtr<CefBrowser>& new_browser) {
    return false;
  }
#endif
```

- Called on the UI thread. `browser` is an Alloy browser of the same profile: the sender if it is a browser, else the active tab of the requested or last focused window, else the first top-level Alloy browser. The handler comes from that browser's client.
- `window_id` is the requested window (`tabs.create` `windowId`), or -1. `new_window` is true for `windows.create`. Only `url`, `window_id`, `active` and `new_window` are passed on.
- Return true if handled. The app should create the browser, call `SetExtensionTabInfo` on it, and set `new_browser`. Then the extension gets a real `Tab` or `Window` object. If `new_browser` stays empty, the extension gets a placeholder (`id` -1) and learns of the real tab from `tabs.onCreated`.
- Return false, or have no `CefLifeSpanHandler`: the extension call fails with an error. This is the default.
- `windows.create` with several URLs: one call for the first URL with `new_window = true`, then one call per extra URL with `new_window = false` and the new window's id.

### How it works

- `p2-cef-api.patch`: `alloy_extension_create_tab.{h,cc}` (`cef::HasAlloyBrowser`, `cef::RequestExtensionCreateTab`), the handler method, and two `BUILD.gn` lines. The `BUILD.gn` lines go after `chrome_mime_handler_view_guest_delegate_cef.h`, the P1 lines before `chrome_extension_util.cc`. They are in different places on purpose so the patches do not conflict.
- `p2-no-native-windows.patch` touches only `extensions/api/tabs/tabs_api.cc`: an include block, a hook in `TabsCreateFunction::Run` before "Try to find a suitable browser", and a hook in `WindowsCreateFunction::Run` before "Decide whether we are opening a normal window or an incognito window".
- Profiles with no Alloy browser run Chromium's code unchanged. Chrome style browsers are not affected.

### Register in `cef/patch/patch.cfg`

Put this directly after the P1 stanza:

```python
  {
    # chrome: Route chrome.tabs.create and chrome.windows.create to
    # CefLifeSpanHandler::OnExtensionCreateTab for Alloy style browsers
    # instead of opening a native window. Our own fork, no upstream issue.
    'name': 'p2-no-native-windows',
  },
```

Copy `p2-no-native-windows.patch` to `cef/patch/patches/p2-no-native-windows.patch`. Apply `p2-cef-api.patch` in the cef checkout, as for P1.

## Apply checks

Run 2026-10-09 from `/Volumes/cefbuild/cef-src/chromium/src` (read-only `git apply -p0 --check`):

| Patch | Directory | Result |
|---|---|---|
| `p1-tab-window-model.patch` | `chromium/src` | OK |
| `p1-cef-api.patch` | `chromium/src/cef` | OK |
| `p2-no-native-windows.patch` | `chromium/src` | OK |
| `p2-cef-api.patch` | `chromium/src/cef` | OK |

Also applied P1 then P2 for both pairs to scratch clones of the original files: no conflict. The tree under `/Volumes/cefbuild` was not modified.

The tree being built already has `chrome_browser_extensions.patch` applied. These patches are made against that state.

## Not done

- No tests. A ceftest for `SetExtensionTabInfo` (register two browsers, run `chrome.tabs.query` from an extension page, check `windowId` and `active`) is the first thing to add once the build works.
- No generated files. `libcef_dll/` wrappers and `include/capi` are produced by the translator in `cef_create_projects`. Nothing was run.
- `tabs.update({active: true})`, `tabs.move`, `tabs.remove`, `tabs.highlight`, `windows.update` do not reach the app. `tabs.remove` on an Alloy tab and `windows.update` return an error or do nothing. A handler per call would follow the P2 pattern.
- `windows.onBoundsChanged` is not fired. There is no bounds data.
- `chrome.runtime.openOptionsPage` is not routed. `OpenOptionsPage` on the Alloy controller returns false, and Chromium only reaches that method through a Browser.
- `tabs.onDetached`/`onAttached`/`onMoved` are not fired when the app moves a tab to another window or reorders it.
- Incognito: the registry matches profiles with `IsSameOrParent`, and a window takes the profile of its first tab. Mixed profiles in one window id are not handled.
- The legacy events `onActiveChanged` and `onSelectionChanged` are not fired.

## Uncertain, needs a compile or a run

1. Nothing was compiled. Expect small errors: missing includes in the new `.cc` files (`gfx::Size` in `alloy_window_controller.cc`, `base::OnceCallback` includes), and `-Wreorder` or unused-variable warnings treated as errors.
2. `raw_ref<Registry>` in `TabEntry` with `Registry` incomplete at that point, and `base::NoDestructor<Registry>` with a private constructor and a friend declaration. If clang rejects either, make the constructor public.
3. `TabEntry::WebContentsDestroyed` destroys the entry through `Registry::Remove` while the `WebContents` is notifying its observers. Chromium's own `TabEntry` in `tabs_event_router.cc` does the same, but this was not checked against 152's `ObserverList` rules.
4. `WebContents::GetContainerBounds()` for the window size: unverified for windowless browsers (the size may be 0 until a first frame).
5. Generated types: `api::tabs::Tab` field names and whether its required fields have default values (the P2 placeholder sets all of them). `api::tabs::ToString(WindowType)` is assumed from `browser_extension_window_controller.cc`. `extension_misc::kId` comes from `extensions/common/constants.h`.
6. `ExtensionFunction::GetSenderWebContents()` is declared public in 152 (it is used in `TabsGetCurrentFunction`), but its result is null for service worker callers. Then the current window falls back to the focused window.
7. The new virtual `WindowController::IsActive()` could clash with a subclass method of the same name elsewhere in Chromium 152 (there are other `WindowController` subclasses, for example app windows). Not checked by grep over the full tree because the checkout was busy compiling.
8. `windows.getLastFocused` and `currentWindow` semantics follow the spec (focused card, locked or not). For a service worker `currentWindow` is the focused window. In Chrome it is the last active browser, which is the same idea.
9. The `CEF_API_ADDED(CEF_EXPERIMENTAL)` blocks follow the `SetAxViewportCollapse` example. `version_manager.py` may want a hash update only if the app selects a stable API version, since this is experimental API. A static method with `added=experimental` was not seen elsewhere in the headers.
10. `cef/libcef/browser/chrome/extensions/` files include `chrome/browser/extensions/...` headers. The existing `chrome_extension_util.cc` does too, so GN deps should be fine, but a `gn check` was not run.
11. P2: `HasAlloyBrowser` uses `CefBrowserInfoManager::GetBrowserInfoList` and so counts any Alloy browser, even one the app did not register as a tab. That is intended: every Alloy app should lose the native window.
