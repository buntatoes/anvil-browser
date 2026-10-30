// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Standard WHATWG DOM, 3-Phase Event Propagation, Fetch/Abort, Traversal,
// Web Crypto, Performance, History, DataView, and 2D Canvas Runtime for Anvil-JS.

(function() {
    var globalObj = typeof window !== 'undefined' ? window : globalThis;

    // =========================================================================
    // 1. Error Reporting & Unhandled Rejection Dispatch (Pillar 1C)
    // =========================================================================
    var unhandledPromises = [];

    function dispatchGlobalError(message, filename, lineno, colno, errorObj) {
        var msg = String(message || 'Error');
        var src = String(filename || (globalObj.location && globalObj.location.href) || '');
        var line = typeof lineno === 'number' ? lineno : 1;
        var col = typeof colno === 'number' ? colno : 1;
        var err = errorObj || new Error(msg);
        if (typeof globalObj.onerror === 'function') {
            try {
                globalObj.onerror(msg, src, line, col, err);
            } catch (_e) {}
        }
        if (typeof globalObj.dispatchEvent === 'function' && typeof ErrorEvent === 'function') {
            try {
                var ev = new ErrorEvent('error', {
                    message: msg,
                    filename: src,
                    lineno: line,
                    colno: col,
                    error: err,
                    cancelable: true
                });
                globalObj.dispatchEvent(ev);
            } catch (_e2) {}
        }
    }

    function dispatchUnhandledRejection(reason, promiseObj) {
        if (typeof globalObj.onunhandledrejection === 'function') {
            try {
                var ev0 = new PromiseRejectionEvent('unhandledrejection', {
                    promise: promiseObj || null,
                    reason: reason,
                    cancelable: true
                });
                globalObj.onunhandledrejection(ev0);
            } catch (_e) {}
        }
        if (typeof globalObj.dispatchEvent === 'function' && typeof PromiseRejectionEvent === 'function') {
            try {
                var ev = new PromiseRejectionEvent('unhandledrejection', {
                    promise: promiseObj || null,
                    reason: reason,
                    cancelable: true
                });
                globalObj.dispatchEvent(ev);
            } catch (_e2) {}
        }
    }

    globalObj.__reportError = dispatchGlobalError;
    globalObj.__reportUncaughtError = dispatchGlobalError;
    globalObj.__reportUnhandledRejection = dispatchUnhandledRejection;
    globalObj.__queueUnhandledRejection = function(reason, p) {
        unhandledPromises.push({ reason: reason, promise: p });
    };
    globalObj.__flushUnhandledRejections = function() {
        while (unhandledPromises.length > 0) {
            var item = unhandledPromises.shift();
            dispatchUnhandledRejection(item.reason, item.promise);
        }
    };

    // Hook Promise.reject to track unhandled rejections if no .catch/.then rejection handler attaches
    if (typeof Promise === 'function' && typeof Promise.reject === 'function') {
        var origReject = Promise.reject.bind(Promise);
        var origThen = Promise.prototype.then;
        var origCatch = Promise.prototype.catch;
        Promise.prototype.then = function(onFulfilled, onRejected) {
            if (typeof onRejected === 'function') {
                this._handled = true;
            }
            var next = origThen.call(this, onFulfilled, onRejected);
            if (next && typeof next === 'object') next._handled = Boolean(this._handled);
            return next;
        };
        Promise.prototype.catch = function(onRejected) {
            this._handled = true;
            var next = origCatch.call(this, onRejected);
            if (next && typeof next === 'object') next._handled = true;
            return next;
        };
        Promise.reject = function(reason) {
            var p = origReject(reason);
            if (p && typeof p === 'object') {
                p._handled = false;
                queueMicrotask(function() {
                    if (!p._handled) {
                        dispatchUnhandledRejection(reason, p);
                    }
                });
            }
            return p;
        };
    }

    // =========================================================================
    // 2. W3C 3-Phase Event Hierarchy & Propagation (Pillar 4B)
    // =========================================================================
    class Event {
        constructor(type, init) {
            this.type = String(type || '');
            this.bubbles = Boolean(init && init.bubbles);
            this.cancelable = Boolean(init && init.cancelable);
            this.composed = Boolean(init && init.composed);
            this.defaultPrevented = false;
            this.target = null;
            this.srcElement = null;
            this.currentTarget = null;
            this.eventPhase = 0;
            this.isTrusted = Boolean(init && init.isTrusted);
            this.timeStamp = (typeof performance !== 'undefined' && typeof performance.now === 'function')
                ? performance.now()
                : Date.now();
            this._stopped = false;
            this._immediateStopped = false;
            this._passive = false;
            this._path = [];
        }
        preventDefault() {
            if (this.cancelable && !this._passive) {
                this.defaultPrevented = true;
                if (typeof globalObj.__nativePreventDefault === 'function') {
                    globalObj.__nativePreventDefault();
                }
            }
        }
        stopPropagation() {
            this._stopped = true;
            if (typeof globalObj.__nativeStopPropagation === 'function') {
                globalObj.__nativeStopPropagation();
            }
        }
        stopImmediatePropagation() {
            this._stopped = true;
            this._immediateStopped = true;
            if (typeof globalObj.__nativeStopPropagation === 'function') {
                globalObj.__nativeStopPropagation();
            }
        }
        composedPath() {
            return Array.isArray(this._path) ? this._path.slice() : [];
        }
    }
    Event.NONE = 0;
    Event.CAPTURING_PHASE = 1;
    Event.AT_TARGET = 2;
    Event.BUBBLING_PHASE = 3;
    Event.prototype.NONE = 0;
    Event.prototype.CAPTURING_PHASE = 1;
    Event.prototype.AT_TARGET = 2;
    Event.prototype.BUBBLING_PHASE = 3;

    class CustomEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.detail = (init && 'detail' in init) ? init.detail : null;
        }
    }

    class UIEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.view = (init && init.view) || null;
            this.detail = (init && typeof init.detail === 'number') ? init.detail : 0;
        }
    }

    class MouseEvent extends UIEvent {
        constructor(type, init) {
            super(type, init);
            this.clientX = (init && typeof init.clientX === 'number') ? init.clientX : 0;
            this.clientY = (init && typeof init.clientY === 'number') ? init.clientY : 0;
            this.pageX = (init && typeof init.pageX === 'number') ? init.pageX : this.clientX;
            this.pageY = (init && typeof init.pageY === 'number') ? init.pageY : this.clientY;
            this.screenX = (init && typeof init.screenX === 'number') ? init.screenX : 0;
            this.screenY = (init && typeof init.screenY === 'number') ? init.screenY : 0;
            this.button = (init && typeof init.button === 'number') ? init.button : 0;
            this.buttons = (init && typeof init.buttons === 'number') ? init.buttons : 0;
            this.ctrlKey = Boolean(init && init.ctrlKey);
            this.shiftKey = Boolean(init && init.shiftKey);
            this.altKey = Boolean(init && init.altKey);
            this.metaKey = Boolean(init && init.metaKey);
            this.relatedTarget = (init && init.relatedTarget) || null;
        }
        getModifierState(keyArg) {
            if (keyArg === 'Control') return this.ctrlKey;
            if (keyArg === 'Shift') return this.shiftKey;
            if (keyArg === 'Alt') return this.altKey;
            if (keyArg === 'Meta') return this.metaKey;
            return false;
        }
    }

    class KeyboardEvent extends UIEvent {
        constructor(type, init) {
            super(type, init);
            this.key = (init && typeof init.key === 'string') ? init.key : '';
            this.code = (init && typeof init.code === 'string') ? init.code : this.key;
            this.location = (init && typeof init.location === 'number') ? init.location : 0;
            this.ctrlKey = Boolean(init && init.ctrlKey);
            this.shiftKey = Boolean(init && init.shiftKey);
            this.altKey = Boolean(init && init.altKey);
            this.metaKey = Boolean(init && init.metaKey);
            this.repeat = Boolean(init && init.repeat);
            this.isComposing = Boolean(init && init.isComposing);
            this.keyCode = (init && typeof init.keyCode === 'number')
                ? init.keyCode
                : (this.key.length === 1 ? this.key.charCodeAt(0) : 0);
        }
        getModifierState(keyArg) {
            if (keyArg === 'Control') return this.ctrlKey;
            if (keyArg === 'Shift') return this.shiftKey;
            if (keyArg === 'Alt') return this.altKey;
            if (keyArg === 'Meta') return this.metaKey;
            return false;
        }
    }

    class InputEvent extends UIEvent {
        constructor(type, init) {
            super(type, init);
            this.data = (init && 'data' in init) ? init.data : null;
            this.inputType = (init && init.inputType) ? String(init.inputType) : 'insertText';
            this.isComposing = Boolean(init && init.isComposing);
        }
    }

    class FocusEvent extends UIEvent {
        constructor(type, init) {
            super(type, init);
            this.relatedTarget = (init && init.relatedTarget) || null;
        }
    }

    class ErrorEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.message = (init && init.message) ? String(init.message) : '';
            this.filename = (init && init.filename) ? String(init.filename) : '';
            this.lineno = (init && typeof init.lineno === 'number') ? init.lineno : 0;
            this.colno = (init && typeof init.colno === 'number') ? init.colno : 0;
            this.error = (init && 'error' in init) ? init.error : null;
        }
    }

    class PromiseRejectionEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.promise = (init && init.promise) || null;
            this.reason = (init && 'reason' in init) ? init.reason : undefined;
        }
    }

    class PopStateEvent extends Event {
        constructor(type, init) {
            super(type, init);
            this.state = (init && 'state' in init) ? init.state : null;
        }
    }

    // Listener storage keyed by target object identity
    var listenerBuckets = [];
    function getBucket(target, create) {
        for (var i = 0; i < listenerBuckets.length; i++) {
            if (listenerBuckets[i].target === target) {
                return listenerBuckets[i].list;
            }
        }
        if (!create) return null;
        var list = [];
        listenerBuckets.push({ target: target, list: list });
        return list;
    }

    function parseListenerOptions(options) {
        if (typeof options === 'boolean') {
            return { capture: options, once: false, passive: false, signal: null };
        }
        if (options && typeof options === 'object') {
            return {
                capture: Boolean(options.capture),
                once: Boolean(options.once),
                passive: Boolean(options.passive),
                signal: options.signal || null
            };
        }
        return { capture: false, once: false, passive: false, signal: null };
    }

    function sameCallback(a, b) {
        return a === b;
    }

    function registerListenerOnTarget(target, type, callback, options) {
        if (!target || !callback) return;
        if (typeof callback !== 'function' && typeof callback.handleEvent !== 'function') return;
        var evType = String(type || '').trim();
        if (!evType) return;
        var opts = parseListenerOptions(options);
        if (opts.signal && opts.signal.aborted) {
            return;
        }
        var lower = evType.toLowerCase();
        if ((target === document || target === globalObj) &&
            (lower === 'domcontentloaded' || lower === 'load' || lower === 'readystatechange')) {
            queueMicrotask(function() {
                var loadEv = new Event(evType, { bubbles: false, cancelable: false });
                loadEv.target = document;
                loadEv.currentTarget = target;
                loadEv.eventPhase = 2;
                if (typeof callback === 'function') {
                    callback.call(target, loadEv);
                } else if (typeof callback.handleEvent === 'function') {
                    callback.handleEvent(loadEv);
                }
            });
            return;
        }
        var list = getBucket(target, true);
        for (var i = 0; i < list.length; i++) {
            var entry = list[i];
            if (entry.type === evType && sameCallback(entry.callback, callback) && entry.capture === opts.capture) {
                return;
            }
        }
        var record = {
            type: evType,
            lowerType: lower,
            callback: callback,
            capture: opts.capture,
            once: opts.once,
            passive: opts.passive,
            removed: false
        };
        list.push(record);
        if (opts.signal && typeof opts.signal.addEventListener === 'function') {
            opts.signal.addEventListener('abort', function() {
                removeListenerFromTarget(target, evType, callback, opts.capture);
            }, { once: true });
        }
        if (typeof target.__nativeAddListener === 'function') {
            try {
                target.__nativeAddListener(lower, function() {});
            } catch (_e) {}
        }
    }

    function removeListenerFromTarget(target, type, callback, options) {
        if (!target || !callback) return;
        var list = getBucket(target, false);
        if (!list) return;
        var evType = String(type || '').trim();
        var opts = parseListenerOptions(options);
        for (var i = list.length - 1; i >= 0; i--) {
            var entry = list[i];
            if (entry.type === evType && sameCallback(entry.callback, callback) && entry.capture === opts.capture) {
                entry.removed = true;
                list.splice(i, 1);
            }
        }
    }

    function buildPropagationPath(target) {
        var path = [target];
        if (!target) return path;
        if (target === globalObj) {
            return path;
        }
        if (target === document) {
            path.push(globalObj);
            return path;
        }
        var cur = target.parentNode || target.parentElement || null;
        var guard = 0;
        while (cur && guard < 256) {
            path.push(cur);
            if (cur === document) break;
            cur = cur.parentNode || cur.parentElement || null;
            guard++;
        }
        if (path[path.length - 1] !== document && target._isDomNode) {
            if (target.isConnected) {
                path.push(document);
                path.push(globalObj);
            }
        } else if (path[path.length - 1] === document) {
            path.push(globalObj);
        }
        return path;
    }

    function invokeListenersAt(currentTarget, eventObj, phase) {
        var list = getBucket(currentTarget, false);
        var onPropName = 'on' + String(eventObj.type || '').toLowerCase();
        var inlineHandler = ((phase === 2 || phase === 3) &&
            !(currentTarget === globalObj && (onPropName === 'onerror' || onPropName === 'onunhandledrejection')))
            ? currentTarget[onPropName]
            : null;
        if ((!list || list.length === 0) && typeof inlineHandler !== 'function') {
            return false;
        }
        var snapshot = list ? list.slice() : [];
        var invokedAny = false;
        eventObj.currentTarget = currentTarget;
        eventObj.eventPhase = phase;

        for (var i = 0; i < snapshot.length; i++) {
            if (eventObj._immediateStopped) break;
            var entry = snapshot[i];
            if (entry.removed) continue;
            var typeMatches = entry.type === eventObj.type ||
                entry.lowerType === String(eventObj.type || '').toLowerCase();
            if (!typeMatches) continue;
            if (phase === 1 && !entry.capture) continue;
            if (phase === 3 && entry.capture) continue;

            if (entry.once) {
                entry.removed = true;
                var liveList = getBucket(currentTarget, false);
                if (liveList) {
                    var idx = liveList.indexOf(entry);
                    if (idx !== -1) liveList.splice(idx, 1);
                }
            }
            var prevPassive = eventObj._passive;
            eventObj._passive = Boolean(entry.passive);
            invokedAny = true;
            try {
                var ret;
                if (typeof entry.callback === 'function') {
                    ret = entry.callback.call(currentTarget, eventObj);
                } else if (entry.callback && typeof entry.callback.handleEvent === 'function') {
                    ret = entry.callback.handleEvent.call(entry.callback, eventObj);
                }
                if (ret === false && !eventObj._passive && eventObj.cancelable) {
                    eventObj.preventDefault();
                }
            } catch (err) {
                dispatchGlobalError(err && err.message ? err.message : String(err), '', 1, 1, err);
            }
            eventObj._passive = prevPassive;
        }

        if (!eventObj._immediateStopped && typeof inlineHandler === 'function') {
            invokedAny = true;
            try {
                var inlineRet = inlineHandler.call(currentTarget, eventObj);
                if (inlineRet === false && eventObj.cancelable) {
                    eventObj.preventDefault();
                }
            } catch (err2) {
                dispatchGlobalError(err2 && err2.message ? err2.message : String(err2), '', 1, 1, err2);
            }
        }
        return invokedAny;
    }

    function dispatchThreePhaseEvent(target, eventObj) {
        if (!target || !eventObj) return true;
        eventObj.target = target;
        eventObj.srcElement = target;
        eventObj._stopped = false;
        eventObj._immediateStopped = false;
        var path = buildPropagationPath(target);
        eventObj._path = path;
        var anyHandled = false;

        // Phase 1: CAPTURING_PHASE (root down to target's parent)
        for (var i = path.length - 1; i >= 1; i--) {
            if (eventObj._stopped) break;
            if (invokeListenersAt(path[i], eventObj, 1)) {
                anyHandled = true;
            }
        }

        // Phase 2: AT_TARGET
        if (!eventObj._stopped) {
            if (invokeListenersAt(target, eventObj, 2)) {
                anyHandled = true;
            }
        }

        // Phase 3: BUBBLING_PHASE (target's parent up to root)
        if (eventObj.bubbles && !eventObj._stopped) {
            for (var j = 1; j < path.length; j++) {
                if (eventObj._stopped) break;
                if (invokeListenersAt(path[j], eventObj, 3)) {
                    anyHandled = true;
                }
            }
        }

        eventObj.currentTarget = null;
        eventObj.eventPhase = 0;
        eventObj._lastHandled = anyHandled;
        return !eventObj.defaultPrevented;
    }

    class EventTarget {
        addEventListener(type, callback, options) {
            registerListenerOnTarget(this, type, callback, options);
        }
        removeEventListener(type, callback, options) {
            removeListenerFromTarget(this, type, callback, options);
        }
        dispatchEvent(eventObj) {
            return dispatchThreePhaseEvent(this, eventObj);
        }
    }

    // Wire 3-phase event methods onto window, document, and Element.prototype
    globalObj.addEventListener = function(type, cb, opts) {
        registerListenerOnTarget(globalObj, type, cb, opts);
    };
    globalObj.removeEventListener = function(type, cb, opts) {
        removeListenerFromTarget(globalObj, type, cb, opts);
    };
    globalObj.dispatchEvent = function(ev) {
        return dispatchThreePhaseEvent(globalObj, ev);
    };

    if (typeof document !== 'undefined') {
        document.__nativeAddListener = document.addEventListener;
        document.addEventListener = function(type, cb, opts) {
            registerListenerOnTarget(document, type, cb, opts);
        };
        document.removeEventListener = function(type, cb, opts) {
            removeListenerFromTarget(document, type, cb, opts);
        };
        document.dispatchEvent = function(ev) {
            return dispatchThreePhaseEvent(document, ev);
        };
    }

    var elProto = globalObj.__elementProto;
    if (elProto) {
        elProto._isDomNode = true;
        elProto.__nativeAddListener = elProto.addEventListener;
        elProto.addEventListener = function(type, cb, opts) {
            registerListenerOnTarget(this, type, cb, opts);
        };
        elProto.removeEventListener = function(type, cb, opts) {
            removeListenerFromTarget(this, type, cb, opts);
        };
        elProto.dispatchEvent = function(ev) {
            return dispatchThreePhaseEvent(this, ev);
        };
    }

    // Bridge for native Rust host event dispatch (`dispatch_click`, `dispatch_input`, etc.)
    globalObj.__dispatchNativeDomEvent = function(targetNode, eventType, valueStr, keyStr) {
        var ev;
        var lower = String(eventType || '').toLowerCase();
        if (lower === 'click' || lower === 'mousedown' || lower === 'mouseup') {
            ev = new MouseEvent(lower, { bubbles: true, cancelable: true, isTrusted: true });
        } else if (lower === 'keydown' || lower === 'keyup' || lower === 'keypress') {
            ev = new KeyboardEvent(lower, {
                bubbles: true,
                cancelable: true,
                isTrusted: true,
                key: keyStr || '',
                code: keyStr || ''
            });
        } else if (lower === 'input') {
            ev = new InputEvent('input', {
                bubbles: true,
                cancelable: false,
                isTrusted: true,
                data: valueStr || ''
            });
        } else {
            ev = new Event(lower, { bubbles: true, cancelable: true, isTrusted: true });
        }
        if (valueStr !== undefined && valueStr !== null) {
            ev.value = valueStr;
            ev.data = valueStr;
        }
        if (keyStr !== undefined && keyStr !== null) {
            ev.key = keyStr;
            ev.code = keyStr;
        }
        dispatchThreePhaseEvent(targetNode || document, ev);
        if (lower === 'input') {
            var changeEv = new Event('change', { bubbles: true, cancelable: false, isTrusted: true });
            changeEv.value = valueStr || '';
            dispatchThreePhaseEvent(targetNode || document, changeEv);
            if (changeEv._lastHandled) ev._lastHandled = true;
        }
        return Boolean(ev._lastHandled);
    };

    // =========================================================================
    // 3. AbortController & AbortSignal (Pillar 4B)
    // =========================================================================
    class AbortSignal extends EventTarget {
        constructor() {
            super();
            this.aborted = false;
            this.reason = undefined;
            this.onabort = null;
        }
        throwIfAborted() {
            if (this.aborted) {
                throw this.reason;
            }
        }
        static abort(reason) {
            var ctrl = new AbortController();
            ctrl.abort(reason);
            return ctrl.signal;
        }
        static timeout(ms) {
            var ctrl = new AbortController();
            setTimeout(function() {
                var err = new Error('The operation timed out.');
                err.name = 'TimeoutError';
                ctrl.abort(err);
            }, typeof ms === 'number' ? ms : 0);
            return ctrl.signal;
        }
        static any(signals) {
            var ctrl = new AbortController();
            var list = Array.isArray(signals) ? signals : [];
            for (var i = 0; i < list.length; i++) {
                var sig = list[i];
                if (sig && sig.aborted) {
                    ctrl.abort(sig.reason);
                    return ctrl.signal;
                }
                if (sig && typeof sig.addEventListener === 'function') {
                    (function(s) {
                        s.addEventListener('abort', function() {
                            ctrl.abort(s.reason);
                        }, { once: true });
                    })(sig);
                }
            }
            return ctrl.signal;
        }
    }

    class AbortController {
        constructor() {
            this.signal = new AbortSignal();
        }
        abort(reason) {
            if (this.signal.aborted) return;
            this.signal.aborted = true;
            if (reason !== undefined) {
                this.signal.reason = reason;
            } else {
                var err = new Error('The operation was aborted.');
                err.name = 'AbortError';
                this.signal.reason = err;
            }
            var ev = new Event('abort', { bubbles: false, cancelable: false });
            this.signal.dispatchEvent(ev);
        }
    }

    // =========================================================================
    // 4. Complete Element / Node / Document Manipulation & Traversal (Pillar 4B)
    // =========================================================================
    function coerceNode(item) {
        if (item && typeof item === 'object' && item._isDomNode) {
            return item;
        }
        return document.createTextNode(String(item === undefined ? '' : item));
    }

    if (elProto) {
        elProto.prepend = function() {
            var first = this.firstChild;
            for (var i = 0; i < arguments.length; i++) {
                var node = coerceNode(arguments[i]);
                if (first) {
                    this.insertBefore(node, first);
                } else {
                    this.appendChild(node);
                }
            }
        };
        elProto.append = function() {
            for (var i = 0; i < arguments.length; i++) {
                this.appendChild(coerceNode(arguments[i]));
            }
        };
        elProto.before = function() {
            var parent = this.parentNode;
            if (!parent) return;
            for (var i = 0; i < arguments.length; i++) {
                parent.insertBefore(coerceNode(arguments[i]), this);
            }
        };
        elProto.after = function() {
            var parent = this.parentNode;
            if (!parent) return;
            var next = this.nextSibling;
            for (var i = 0; i < arguments.length; i++) {
                var node = coerceNode(arguments[i]);
                if (next) {
                    parent.insertBefore(node, next);
                } else {
                    parent.appendChild(node);
                }
            }
        };
        elProto.replaceWith = function() {
            var parent = this.parentNode;
            if (!parent) return;
            var next = this.nextSibling;
            this.remove();
            for (var i = 0; i < arguments.length; i++) {
                var node = coerceNode(arguments[i]);
                if (next) {
                    parent.insertBefore(node, next);
                } else {
                    parent.appendChild(node);
                }
            }
        };
        elProto.replaceChildren = function() {
            while (this.firstChild) {
                this.removeChild(this.firstChild);
            }
            for (var i = 0; i < arguments.length; i++) {
                this.appendChild(coerceNode(arguments[i]));
            }
        };
        elProto.toggleAttribute = function(name, force) {
            var attr = String(name || '');
            var has = this.hasAttribute(attr);
            if (force === undefined) {
                if (has) {
                    this.removeAttribute(attr);
                    return false;
                }
                this.setAttribute(attr, '');
                return true;
            }
            if (Boolean(force)) {
                if (!has) this.setAttribute(attr, '');
                return true;
            }
            if (has) this.removeAttribute(attr);
            return false;
        };
        elProto.hasChildNodes = function() {
            return Boolean(this.firstChild);
        };
        elProto.getRootNode = function() {
            var cur = this;
            while (cur.parentNode) {
                cur = cur.parentNode;
            }
            return cur;
        };
        elProto.isSameNode = function(other) {
            return this === other;
        };
        elProto.isEqualNode = function(other) {
            if (!other || typeof other !== 'object') return false;
            if (this === other) return true;
            return this.nodeType === other.nodeType &&
                this.nodeName === other.nodeName &&
                this.outerHTML === other.outerHTML;
        };
        elProto.compareDocumentPosition = function(other) {
            if (!other || this === other) return 0;
            if (this.contains(other)) return 16 | 4; // DOCUMENT_POSITION_CONTAINED_BY | FOLLOWING
            if (other.contains && other.contains(this)) return 8 | 2; // CONTAINS | PRECEDING
            var all = document.querySelectorAll('*');
            var idxA = all.indexOf(this);
            var idxB = all.indexOf(other);
            if (idxA !== -1 && idxB !== -1) {
                return idxA < idxB ? 4 : 2;
            }
            return 1; // DISCONNECTED
        };
        elProto.normalize = function() {
            var kids = this.childNodes;
            var accText = null;
            for (var i = 0; i < kids.length; i++) {
                var child = kids[i];
                if (child.nodeType === 3) {
                    var val = child.textContent || '';
                    if (!val) {
                        child.remove();
                    } else if (accText) {
                        accText.textContent = (accText.textContent || '') + val;
                        child.remove();
                    } else {
                        accText = child;
                    }
                } else {
                    accText = null;
                    if (typeof child.normalize === 'function') child.normalize();
                }
            }
        };
    }

    var NodeFilter = {
        FILTER_ACCEPT: 1,
        FILTER_REJECT: 2,
        FILTER_SKIP: 3,
        SHOW_ALL: 0xFFFFFFFF,
        SHOW_ELEMENT: 0x1,
        SHOW_TEXT: 0x4,
        SHOW_COMMENT: 0x80,
        SHOW_DOCUMENT: 0x100
    };

    function collectSubtreeNodes(root, whatToShow, filter) {
        var mask = (whatToShow === undefined || whatToShow === null) ? 0xFFFFFFFF : (whatToShow >>> 0);
        var out = [];
        function matchesMask(node) {
            var nt = node.nodeType || 1;
            if (mask === 0xFFFFFFFF) return true;
            if (nt === 1 && (mask & 0x1)) return true;
            if (nt === 3 && (mask & 0x4)) return true;
            if (nt === 8 && (mask & 0x80)) return true;
            if (nt === 9 && (mask & 0x100)) return true;
            return false;
        }
        function checkFilter(node) {
            if (!matchesMask(node)) return 3;
            if (!filter) return 1;
            if (typeof filter === 'function') return filter(node);
            if (typeof filter.acceptNode === 'function') return filter.acceptNode(node);
            return 1;
        }
        function walk(node) {
            var verdict = checkFilter(node);
            if (verdict === 2) return; // FILTER_REJECT skips subtree
            if (verdict === 1) out.push(node);
            var kids = node.childNodes || [];
            for (var i = 0; i < kids.length; i++) {
                walk(kids[i]);
            }
        }
        if (root) walk(root);
        return out;
    }

    class TreeWalker {
        constructor(root, whatToShow, filter) {
            this.root = root;
            this.whatToShow = (whatToShow === undefined) ? 0xFFFFFFFF : whatToShow;
            this.filter = filter || null;
            this.currentNode = root;
        }
        _nodes() {
            return collectSubtreeNodes(this.root, this.whatToShow, this.filter);
        }
        nextNode() {
            var list = this._nodes();
            var idx = list.indexOf(this.currentNode);
            if (idx === -1) {
                if (list.length > 0 && this.currentNode === this.root) {
                    this.currentNode = list[0];
                    return this.currentNode;
                }
                return null;
            }
            if (idx + 1 < list.length) {
                this.currentNode = list[idx + 1];
                return this.currentNode;
            }
            return null;
        }
        previousNode() {
            var list = this._nodes();
            var idx = list.indexOf(this.currentNode);
            if (idx > 0) {
                this.currentNode = list[idx - 1];
                return this.currentNode;
            }
            return null;
        }
        firstChild() {
            var list = this._nodes();
            for (var i = 0; i < list.length; i++) {
                if (list[i].parentNode === this.currentNode) {
                    this.currentNode = list[i];
                    return this.currentNode;
                }
            }
            return null;
        }
        lastChild() {
            var list = this._nodes();
            for (var i = list.length - 1; i >= 0; i--) {
                if (list[i].parentNode === this.currentNode) {
                    this.currentNode = list[i];
                    return this.currentNode;
                }
            }
            return null;
        }
        parentNode() {
            var p = this.currentNode && this.currentNode.parentNode;
            if (p && this.root && (p === this.root || this.root.contains(p))) {
                this.currentNode = p;
                return p;
            }
            return null;
        }
    }

    class NodeIterator {
        constructor(root, whatToShow, filter) {
            this.root = root;
            this.whatToShow = (whatToShow === undefined) ? 0xFFFFFFFF : whatToShow;
            this.filter = filter || null;
            this._nodes = collectSubtreeNodes(root, this.whatToShow, this.filter);
            this._index = 0;
            this.referenceNode = root;
            this.pointerBeforeReferenceNode = true;
        }
        nextNode() {
            if (this._index < this._nodes.length) {
                var n = this._nodes[this._index++];
                this.referenceNode = n;
                this.pointerBeforeReferenceNode = false;
                return n;
            }
            return null;
        }
        previousNode() {
            if (this._index > 0) {
                var n = this._nodes[--this._index];
                this.referenceNode = n;
                this.pointerBeforeReferenceNode = true;
                return n;
            }
            return null;
        }
        detach() {}
    }

    class Range {
        constructor() {
            var root = (typeof document !== 'undefined' && document.body) || null;
            this.startContainer = root;
            this.startOffset = 0;
            this.endContainer = root;
            this.endOffset = 0;
            this.collapsed = true;
            this.commonAncestorContainer = root;
        }
        setStart(node, offset) {
            this.startContainer = node;
            this.startOffset = offset || 0;
            this._update();
        }
        setEnd(node, offset) {
            this.endContainer = node;
            this.endOffset = offset || 0;
            this._update();
        }
        selectNode(node) {
            var parent = (node && node.parentNode) || node;
            this.startContainer = parent;
            this.endContainer = parent;
            this.startOffset = 0;
            this.endOffset = 1;
            this._selectedNode = node;
            this.collapsed = false;
            this.commonAncestorContainer = parent;
        }
        selectNodeContents(node) {
            this.startContainer = node;
            this.endContainer = node;
            this.startOffset = 0;
            var kids = (node && node.childNodes) ? node.childNodes.length : 0;
            this.endOffset = kids || (node && node.textContent ? node.textContent.length : 0);
            this._selectedNode = node;
            this.collapsed = this.startOffset === this.endOffset;
            this.commonAncestorContainer = node;
        }
        collapse(toStart) {
            if (toStart) {
                this.endContainer = this.startContainer;
                this.endOffset = this.startOffset;
            } else {
                this.startContainer = this.endContainer;
                this.startOffset = this.endOffset;
            }
            this.collapsed = true;
        }
        cloneRange() {
            var r = new Range();
            r.startContainer = this.startContainer;
            r.startOffset = this.startOffset;
            r.endContainer = this.endContainer;
            r.endOffset = this.endOffset;
            r.collapsed = this.collapsed;
            r.commonAncestorContainer = this.commonAncestorContainer;
            r._selectedNode = this._selectedNode;
            return r;
        }
        cloneContents() {
            var frag = document.createDocumentFragment();
            if (this._selectedNode) {
                frag.appendChild(this._selectedNode.cloneNode(true));
            }
            return frag;
        }
        deleteContents() {
            if (this._selectedNode && this._selectedNode !== document.body) {
                this._selectedNode.remove();
            }
        }
        extractContents() {
            var frag = this.cloneContents();
            this.deleteContents();
            return frag;
        }
        insertNode(node) {
            if (this.startContainer && typeof this.startContainer.insertBefore === 'function') {
                this.startContainer.insertBefore(node, this.startContainer.firstChild);
            }
        }
        getBoundingClientRect() {
            return { x: 0, y: 0, top: 0, left: 0, width: 100, height: 20, right: 100, bottom: 20 };
        }
        toString() {
            if (this._selectedNode) {
                return this._selectedNode.textContent || '';
            }
            if (this.startContainer && this.startContainer === this.endContainer && this.startContainer.nodeType === 3) {
                return (this.startContainer.textContent || '').slice(this.startOffset, this.endOffset);
            }
            return (this.startContainer && this.startContainer.textContent) || '';
        }
        _update() {
            this.collapsed = (this.startContainer === this.endContainer && this.startOffset === this.endOffset);
            this.commonAncestorContainer = this.startContainer;
        }
    }

    var activeSelectionRanges = [];
    var selectionObj = {
        get rangeCount() { return activeSelectionRanges.length; },
        get isCollapsed() { return activeSelectionRanges.length === 0 || activeSelectionRanges[0].collapsed; },
        get anchorNode() { return activeSelectionRanges.length > 0 ? activeSelectionRanges[0].startContainer : null; },
        get anchorOffset() { return activeSelectionRanges.length > 0 ? activeSelectionRanges[0].startOffset : 0; },
        get focusNode() { return activeSelectionRanges.length > 0 ? activeSelectionRanges[0].endContainer : null; },
        get focusOffset() { return activeSelectionRanges.length > 0 ? activeSelectionRanges[0].endOffset : 0; },
        addRange: function(r) { if (r) activeSelectionRanges = [r]; },
        removeAllRanges: function() { activeSelectionRanges = []; },
        empty: function() { activeSelectionRanges = []; },
        getRangeAt: function(i) { return activeSelectionRanges[i || 0] || null; },
        selectAllChildren: function(node) {
            var r = new Range();
            r.selectNodeContents(node);
            activeSelectionRanges = [r];
        },
        toString: function() {
            return activeSelectionRanges.length > 0 ? activeSelectionRanges[0].toString() : '';
        }
    };

    if (typeof document !== 'undefined') {
        document.createTreeWalker = function(root, whatToShow, filter) {
            return new TreeWalker(root, whatToShow, filter);
        };
        document.createNodeIterator = function(root, whatToShow, filter) {
            return new NodeIterator(root, whatToShow, filter);
        };
        document.createRange = function() {
            return new Range();
        };
        document.getSelection = function() {
            return selectionObj;
        };
    }
    globalObj.getSelection = function() {
        return selectionObj;
    };

    // =========================================================================
    // 5. Fetch, Headers, Request, Response (Pillar 4B)
    // =========================================================================
    class Headers {
        constructor(init) {
            this._map = [];
            if (!init) return;
            if (init instanceof Headers || (init && Array.isArray(init._map))) {
                for (var i = 0; i < init._map.length; i++) {
                    this._map.push([init._map[i][0], init._map[i][1]]);
                }
            } else if (Array.isArray(init)) {
                for (var j = 0; j < init.length; j++) {
                    if (init[j] && init[j].length >= 2) {
                        this.append(init[j][0], init[j][1]);
                    }
                }
            } else if (typeof init === 'object') {
                var keys = Object.keys(init);
                for (var k = 0; k < keys.length; k++) {
                    this.append(keys[k], init[keys[k]]);
                }
            }
        }
        append(name, value) {
            var k = String(name || '').trim().toLowerCase();
            var v = String(value === undefined ? '' : value).trim();
            for (var i = 0; i < this._map.length; i++) {
                if (this._map[i][0] === k) {
                    this._map[i][1] = this._map[i][1] + ', ' + v;
                    return;
                }
            }
            this._map.push([k, v]);
        }
        set(name, value) {
            var k = String(name || '').trim().toLowerCase();
            var v = String(value === undefined ? '' : value).trim();
            for (var i = 0; i < this._map.length; i++) {
                if (this._map[i][0] === k) {
                    this._map[i][1] = v;
                    return;
                }
            }
            this._map.push([k, v]);
        }
        get(name) {
            var k = String(name || '').trim().toLowerCase();
            for (var i = 0; i < this._map.length; i++) {
                if (this._map[i][0] === k) return this._map[i][1];
            }
            return null;
        }
        has(name) {
            return this.get(name) !== null;
        }
        delete(name) {
            var k = String(name || '').trim().toLowerCase();
            for (var i = this._map.length - 1; i >= 0; i--) {
                if (this._map[i][0] === k) this._map.splice(i, 1);
            }
        }
        forEach(cb, thisArg) {
            for (var i = 0; i < this._map.length; i++) {
                cb.call(thisArg, this._map[i][1], this._map[i][0], this);
            }
        }
        keys() {
            var out = [];
            for (var i = 0; i < this._map.length; i++) out.push(this._map[i][0]);
            return out;
        }
        values() {
            var out = [];
            for (var i = 0; i < this._map.length; i++) out.push(this._map[i][1]);
            return out;
        }
        entries() {
            return this._map.slice();
        }
    }

    class Request {
        constructor(input, init) {
            if (input instanceof Request) {
                this.url = input.url;
                this.method = (init && init.method) ? String(init.method).toUpperCase() : input.method;
                this.headers = new Headers((init && init.headers) || input.headers);
                this._body = (init && 'body' in init) ? String(init.body || '') : input._body;
                this.signal = (init && init.signal) || input.signal || new AbortSignal();
            } else {
                this.url = String(input || '');
                this.method = (init && init.method) ? String(init.method).toUpperCase() : 'GET';
                this.headers = new Headers(init && init.headers);
                this._body = (init && init.body !== undefined && init.body !== null) ? String(init.body) : '';
                this.signal = (init && init.signal) || new AbortSignal();
            }
            this.bodyUsed = false;
        }
        text() {
            this.bodyUsed = true;
            return Promise.resolve(this._body);
        }
        json() {
            this.bodyUsed = true;
            var raw = this._body;
            return new Promise(function(resolve) {
                resolve(JSON.parse(raw));
            });
        }
        clone() {
            return new Request(this);
        }
    }

    class Response {
        constructor(body, init) {
            this._body = (body === undefined || body === null) ? '' : String(body);
            this.status = (init && typeof init.status === 'number') ? init.status : 200;
            this.statusText = (init && init.statusText) ? String(init.statusText) : (this.status === 200 ? 'OK' : '');
            this.ok = this.status >= 200 && this.status < 300;
            this.headers = new Headers(init && init.headers);
            this.url = (init && init.url) ? String(init.url) : '';
            this.redirected = false;
            this.type = 'basic';
            this.bodyUsed = false;
        }
        text() {
            this.bodyUsed = true;
            return Promise.resolve(this._body);
        }
        json() {
            this.bodyUsed = true;
            var raw = this._body;
            return new Promise(function(resolve) {
                resolve(JSON.parse(raw));
            });
        }
        arrayBuffer() {
            this.bodyUsed = true;
            var s = this._body;
            var buf = new ArrayBuffer(s.length);
            var view = new Uint8Array(buf);
            for (var i = 0; i < s.length; i++) {
                view[i] = s.charCodeAt(i) & 0xFF;
            }
            return Promise.resolve(buf);
        }
        blob() {
            this.bodyUsed = true;
            return Promise.resolve(new Blob([this._body], {
                type: this.headers.get('content-type') || 'text/plain'
            }));
        }
        clone() {
            return new Response(this._body, {
                status: this.status,
                statusText: this.statusText,
                headers: this.headers,
                url: this.url
            });
        }
        static json(data, init) {
            var opts = init ? Object.assign({}, init) : {};
            var h = new Headers(opts.headers);
            if (!h.has('content-type')) h.set('content-type', 'application/json');
            opts.headers = h;
            return new Response(JSON.stringify(data), opts);
        }
        static error() {
            var r = new Response('', { status: 0, statusText: '' });
            r.ok = false;
            r.type = 'error';
            return r;
        }
        static redirect(url, status) {
            return new Response('', {
                status: status || 302,
                headers: { location: String(url || '') }
            });
        }
    }

    globalObj.fetch = function(input, init) {
        var req = (input instanceof Request) ? new Request(input, init) : new Request(input, init);
        var sig = (init && init.signal) || req.signal;
        if (sig && sig.aborted) {
            return Promise.reject(sig.reason || new Error('The operation was aborted.'));
        }
        return new Promise(function(resolve, reject) {
            if (sig && typeof sig.addEventListener === 'function') {
                sig.addEventListener('abort', function() {
                    reject(sig.reason || new Error('The operation was aborted.'));
                }, { once: true });
            }
            if (typeof globalObj.__nativeFetch === 'function') {
                var res = globalObj.__nativeFetch(req.url, req.method, req._body);
                if (sig && sig.aborted) {
                    reject(sig.reason || new Error('The operation was aborted.'));
                    return;
                }
                if (res && typeof res === 'object') {
                    if (res.status === 0) {
                        reject(new TypeError('NetworkError when attempting to fetch resource.'));
                        return;
                    }
                    resolve(new Response(res.body || '', {
                        status: res.status || 200,
                        headers: res.headers || { 'content-type': 'text/plain' },
                        url: res.url || req.url
                    }));
                    return;
                }
            }
            resolve(new Response('', { status: 200, url: req.url }));
        });
    };

    // =========================================================================
    // 6. ArrayBuffer, TypedArrays & DataView (Pillar 2)
    // =========================================================================
    if (typeof globalObj.ArrayBuffer !== 'function') {
        class ArrayBuffer {
            constructor(byteLength) {
                var n = Math.max(0, byteLength | 0);
                this.byteLength = n;
                this._bytes = [];
                for (var i = 0; i < n; i++) this._bytes.push(0);
            }
            slice(begin, end) {
                var b = begin !== undefined ? (begin | 0) : 0;
                var e = end !== undefined ? (end | 0) : this.byteLength;
                if (b < 0) b = Math.max(0, this.byteLength + b);
                if (e < 0) e = Math.max(0, this.byteLength + e);
                var len = Math.max(0, Math.min(this.byteLength, e) - b);
                var out = new ArrayBuffer(len);
                for (var i = 0; i < len; i++) {
                    out._bytes[i] = this._bytes[b + i];
                }
                return out;
            }
            static isView(arg) {
                return Boolean(arg && arg.buffer instanceof ArrayBuffer);
            }
        }
        globalObj.ArrayBuffer = ArrayBuffer;
    }

    function createTypedArrayClass(bytesPerElement) {
        return class TypedArray {
            constructor(arg, byteOffset, length) {
                this.BYTES_PER_ELEMENT = bytesPerElement;
                if (typeof arg === 'number') {
                    var count = Math.max(0, arg | 0);
                    this.buffer = new globalObj.ArrayBuffer(count * bytesPerElement);
                    this.byteOffset = 0;
                    this.byteLength = count * bytesPerElement;
                    this.length = count;
                    for (var i = 0; i < count; i++) this[i] = 0;
                } else if (arg && typeof arg.byteLength === 'number' && Array.isArray(arg._bytes)) {
                    var off = byteOffset !== undefined ? (byteOffset | 0) : 0;
                    var len = length !== undefined ? (length | 0) : Math.floor((arg.byteLength - off) / bytesPerElement);
                    this.buffer = arg;
                    this.byteOffset = off;
                    this.byteLength = len * bytesPerElement;
                    this.length = len;
                    for (var j = 0; j < len; j++) {
                        this[j] = arg._bytes[off + j * bytesPerElement] || 0;
                    }
                } else if (arg && typeof arg.length === 'number') {
                    var n = arg.length | 0;
                    this.buffer = new globalObj.ArrayBuffer(n * bytesPerElement);
                    this.byteOffset = 0;
                    this.byteLength = n * bytesPerElement;
                    this.length = n;
                    for (var k = 0; k < n; k++) {
                        var v = Number(arg[k]) || 0;
                        this[k] = v;
                        this.buffer._bytes[k * bytesPerElement] = v & 0xFF;
                    }
                } else {
                    this.buffer = new globalObj.ArrayBuffer(0);
                    this.byteOffset = 0;
                    this.byteLength = 0;
                    this.length = 0;
                }
            }
            set(source, offset) {
                var off = offset | 0;
                if (!source || typeof source.length !== 'number') return;
                for (var i = 0; i < source.length; i++) {
                    if (off + i < this.length) {
                        this[off + i] = source[i];
                        if (this.buffer && Array.isArray(this.buffer._bytes)) {
                            this.buffer._bytes[this.byteOffset + (off + i) * bytesPerElement] = source[i] & 0xFF;
                        }
                    }
                }
            }
            subarray(begin, end) {
                var b = begin !== undefined ? (begin | 0) : 0;
                var e = end !== undefined ? (end | 0) : this.length;
                if (b < 0) b = Math.max(0, this.length + b);
                if (e < 0) e = Math.max(0, this.length + e);
                var out = [];
                for (var i = b; i < e && i < this.length; i++) out.push(this[i]);
                return new this.constructor(out);
            }
            slice(begin, end) {
                return this.subarray(begin, end);
            }
            fill(value, start, end) {
                var s = start !== undefined ? (start | 0) : 0;
                var e = end !== undefined ? (end | 0) : this.length;
                for (var i = s; i < e && i < this.length; i++) {
                    this[i] = value;
                }
                return this;
            }
        };
    }

    if (typeof globalObj.Uint8Array !== 'function') globalObj.Uint8Array = createTypedArrayClass(1);
    if (typeof globalObj.Uint8ClampedArray !== 'function') globalObj.Uint8ClampedArray = createTypedArrayClass(1);
    if (typeof globalObj.Int8Array !== 'function') globalObj.Int8Array = createTypedArrayClass(1);
    if (typeof globalObj.Uint16Array !== 'function') globalObj.Uint16Array = createTypedArrayClass(2);
    if (typeof globalObj.Int16Array !== 'function') globalObj.Int16Array = createTypedArrayClass(2);
    if (typeof globalObj.Uint32Array !== 'function') globalObj.Uint32Array = createTypedArrayClass(4);
    if (typeof globalObj.Int32Array !== 'function') globalObj.Int32Array = createTypedArrayClass(4);
    if (typeof globalObj.Float32Array !== 'function') globalObj.Float32Array = createTypedArrayClass(4);
    if (typeof globalObj.Float64Array !== 'function') globalObj.Float64Array = createTypedArrayClass(8);

    class DataView {
        constructor(buffer, byteOffset, byteLength) {
            if (!buffer || typeof buffer.byteLength !== 'number') {
                throw new TypeError('First argument to DataView constructor must be an ArrayBuffer');
            }
            var off = byteOffset !== undefined ? (byteOffset | 0) : 0;
            if (off < 0 || off > buffer.byteLength) {
                throw new RangeError('Offset is outside the bounds of the DataView');
            }
            var len = byteLength !== undefined ? (byteLength | 0) : (buffer.byteLength - off);
            if (len < 0 || off + len > buffer.byteLength) {
                throw new RangeError('Invalid DataView length');
            }
            this.buffer = buffer;
            this.byteOffset = off;
            this.byteLength = len;
            if (!Array.isArray(buffer._bytes)) {
                buffer._bytes = [];
                for (var i = 0; i < buffer.byteLength; i++) buffer._bytes.push(0);
            }
            this._u8 = buffer._bytes;
        }
        _check(offset, size) {
            var idx = offset | 0;
            if (idx < 0 || idx + size > this.byteLength) {
                throw new RangeError('Offset is outside the bounds of the DataView');
            }
            return this.byteOffset + idx;
        }
        getUint8(byteOffset) {
            var p = this._check(byteOffset, 1);
            return this._u8[p] & 0xFF;
        }
        getInt8(byteOffset) {
            var v = this.getUint8(byteOffset);
            return v >= 128 ? v - 256 : v;
        }
        setUint8(byteOffset, value) {
            var p = this._check(byteOffset, 1);
            this._u8[p] = (value | 0) & 0xFF;
        }
        setInt8(byteOffset, value) {
            this.setUint8(byteOffset, value);
        }
        getUint16(byteOffset, littleEndian) {
            var p = this._check(byteOffset, 2);
            var b0 = this._u8[p] & 0xFF;
            var b1 = this._u8[p + 1] & 0xFF;
            return littleEndian ? (b0 | (b1 << 8)) : ((b0 << 8) | b1);
        }
        getInt16(byteOffset, littleEndian) {
            var v = this.getUint16(byteOffset, littleEndian);
            return v >= 32768 ? v - 65536 : v;
        }
        setUint16(byteOffset, value, littleEndian) {
            var p = this._check(byteOffset, 2);
            var v = (value | 0) & 0xFFFF;
            if (littleEndian) {
                this._u8[p] = v & 0xFF;
                this._u8[p + 1] = (v >> 8) & 0xFF;
            } else {
                this._u8[p] = (v >> 8) & 0xFF;
                this._u8[p + 1] = v & 0xFF;
            }
        }
        setInt16(byteOffset, value, littleEndian) {
            this.setUint16(byteOffset, value, littleEndian);
        }
        getInt32(byteOffset, littleEndian) {
            var p = this._check(byteOffset, 4);
            var b0 = this._u8[p] & 0xFF;
            var b1 = this._u8[p + 1] & 0xFF;
            var b2 = this._u8[p + 2] & 0xFF;
            var b3 = this._u8[p + 3] & 0xFF;
            if (littleEndian) {
                return (b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)) | 0;
            }
            return ((b0 << 24) | (b1 << 16) | (b2 << 8) | b3) | 0;
        }
        getUint32(byteOffset, littleEndian) {
            return this.getInt32(byteOffset, littleEndian) >>> 0;
        }
        setInt32(byteOffset, value, littleEndian) {
            var p = this._check(byteOffset, 4);
            var v = value | 0;
            if (littleEndian) {
                this._u8[p] = v & 0xFF;
                this._u8[p + 1] = (v >>> 8) & 0xFF;
                this._u8[p + 2] = (v >>> 16) & 0xFF;
                this._u8[p + 3] = (v >>> 24) & 0xFF;
            } else {
                this._u8[p] = (v >>> 24) & 0xFF;
                this._u8[p + 1] = (v >>> 16) & 0xFF;
                this._u8[p + 2] = (v >>> 8) & 0xFF;
                this._u8[p + 3] = v & 0xFF;
            }
        }
        setUint32(byteOffset, value, littleEndian) {
            this.setInt32(byteOffset, value, littleEndian);
        }
        getFloat64(byteOffset, littleEndian) {
            var p = this._check(byteOffset, 8);
            var tmp = new ArrayBuffer(8);
            var u8 = new Uint8Array(tmp);
            for (var i = 0; i < 8; i++) {
                u8[i] = this._u8[p + (littleEndian ? i : (7 - i))];
            }
            var f64 = new Float64Array(tmp);
            return f64[0];
        }
        setFloat64(byteOffset, value, littleEndian) {
            var p = this._check(byteOffset, 8);
            var tmp = new ArrayBuffer(8);
            var f64 = new Float64Array(tmp);
            f64[0] = Number(value);
            var u8 = new Uint8Array(tmp);
            for (var i = 0; i < 8; i++) {
                this._u8[p + (littleEndian ? i : (7 - i))] = u8[i];
            }
        }
    }

    // =========================================================================
    // 7. Web Crypto API, Performance Timeline & History API (Pillar 4B)
    // =========================================================================
    var prngSeed = 0x9E3779B9;
    function nextRandomByte() {
        prngSeed = (prngSeed ^ (prngSeed << 13)) >>> 0;
        prngSeed = (prngSeed ^ (prngSeed >>> 17)) >>> 0;
        prngSeed = (prngSeed ^ (prngSeed << 5)) >>> 0;
        return prngSeed & 0xFF;
    }

    var cryptoObj = {
        getRandomValues: function(typedArray) {
            if (!typedArray || typeof typedArray.length !== 'number') {
                throw new TypeError('Expected an integer TypedArray');
            }
            for (var i = 0; i < typedArray.length; i++) {
                typedArray[i] = nextRandomByte();
            }
            return typedArray;
        },
        randomUUID: function() {
            var hex = '0123456789abcdef';
            var bytes = [];
            for (var i = 0; i < 16; i++) bytes.push(nextRandomByte());
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            var out = '';
            for (var j = 0; j < 16; j++) {
                if (j === 4 || j === 6 || j === 8 || j === 10) out += '-';
                out += hex[(bytes[j] >> 4) & 0x0f] + hex[bytes[j] & 0x0f];
            }
            return out;
        },
        subtle: {
            digest: function(algorithm, data) {
                var alg = typeof algorithm === 'string'
                    ? algorithm.toUpperCase()
                    : String((algorithm && algorithm.name) || 'SHA-256').toUpperCase();
                var bytes = [];
                if (typeof data === 'string') {
                    for (var i = 0; i < data.length; i++) bytes.push(data.charCodeAt(i) & 0xFF);
                } else if (data && typeof data.byteLength === 'number') {
                    var view = data instanceof Uint8Array ? data : new Uint8Array(data.buffer || data);
                    for (var j = 0; j < view.length; j++) bytes.push(view[j] & 0xFF);
                } else if (Array.isArray(data)) {
                    for (var k = 0; k < data.length; k++) bytes.push(data[k] & 0xFF);
                }
                if (alg === 'SHA-256' && typeof globalObj.__sha256Bytes === 'function') {
                    var digestArr = globalObj.__sha256Bytes(bytes);
                    var ab = new ArrayBuffer(digestArr.length);
                    var u8 = new Uint8Array(ab);
                    for (var m = 0; m < digestArr.length; m++) u8[m] = digestArr[m];
                    return Promise.resolve(ab);
                }
                return Promise.reject(new Error('Unsupported digest algorithm: ' + alg));
            }
        }
    };
    globalObj.crypto = cryptoObj;

    var perfEntries = [];
    var perfTimeOrigin = 1700000000000;
    var perfCounter = 0;
    var performanceObj = {
        timeOrigin: perfTimeOrigin,
        now: function() {
            perfCounter += 1;
            return perfCounter;
        },
        mark: function(name, options) {
            var start = (options && typeof options.startTime === 'number') ? options.startTime : performanceObj.now();
            var entry = {
                name: String(name || ''),
                entryType: 'mark',
                startTime: start,
                duration: 0,
                detail: (options && 'detail' in options) ? options.detail : null
            };
            perfEntries.push(entry);
            return entry;
        },
        measure: function(name, startMark, endMark) {
            var start = 0;
            var end = performanceObj.now();
            if (typeof startMark === 'string') {
                for (var i = perfEntries.length - 1; i >= 0; i--) {
                    if (perfEntries[i].name === startMark) {
                        start = perfEntries[i].startTime;
                        break;
                    }
                }
            }
            if (typeof endMark === 'string') {
                for (var j = perfEntries.length - 1; j >= 0; j--) {
                    if (perfEntries[j].name === endMark) {
                        end = perfEntries[j].startTime;
                        break;
                    }
                }
            }
            var entry = {
                name: String(name || ''),
                entryType: 'measure',
                startTime: start,
                duration: Math.max(0, end - start),
                detail: null
            };
            perfEntries.push(entry);
            return entry;
        },
        getEntries: function() {
            return perfEntries.slice();
        },
        getEntriesByName: function(name, type) {
            var out = [];
            for (var i = 0; i < perfEntries.length; i++) {
                if (perfEntries[i].name === name && (!type || perfEntries[i].entryType === type)) {
                    out.push(perfEntries[i]);
                }
            }
            return out;
        },
        getEntriesByType: function(type) {
            var out = [];
            for (var i = 0; i < perfEntries.length; i++) {
                if (perfEntries[i].entryType === type) out.push(perfEntries[i]);
            }
            return out;
        },
        clearMarks: function(name) {
            for (var i = perfEntries.length - 1; i >= 0; i--) {
                if (perfEntries[i].entryType === 'mark' && (!name || perfEntries[i].name === name)) {
                    perfEntries.splice(i, 1);
                }
            }
        },
        clearMeasures: function(name) {
            for (var i = perfEntries.length - 1; i >= 0; i--) {
                if (perfEntries[i].entryType === 'measure' && (!name || perfEntries[i].name === name)) {
                    perfEntries.splice(i, 1);
                }
            }
        }
    };
    globalObj.performance = performanceObj;

    // History stack with pushState / replaceState / back / forward / go / PopStateEvent
    if (globalObj.history) {
        var histStack = [{
            state: null,
            title: '',
            url: (globalObj.location && globalObj.location.href) || 'https://localhost/'
        }];
        var histIdx = 0;
        var nativePush = globalObj.history.pushState;
        Object.defineProperty(globalObj.history, 'state', {
            enumerable: true,
            configurable: true,
            get: function() { return histStack[histIdx] ? histStack[histIdx].state : null; }
        });
        Object.defineProperty(globalObj.history, 'length', {
            enumerable: true,
            configurable: true,
            get: function() { return histStack.length; }
        });
        globalObj.history.pushState = function(state, title, url) {
            var cloned = (state !== undefined && typeof structuredClone === 'function')
                ? structuredClone(state)
                : state;
            if (url !== undefined && url !== null && typeof nativePush === 'function') {
                nativePush.call(globalObj.history, cloned, title || '', String(url));
            }
            histStack = histStack.slice(0, histIdx + 1);
            histStack.push({
                state: cloned === undefined ? null : cloned,
                title: String(title || ''),
                url: (globalObj.location && globalObj.location.href) || String(url || '')
            });
            histIdx = histStack.length - 1;
        };
        globalObj.history.replaceState = function(state, title, url) {
            var cloned = (state !== undefined && typeof structuredClone === 'function')
                ? structuredClone(state)
                : state;
            if (url !== undefined && url !== null && typeof nativePush === 'function') {
                nativePush.call(globalObj.history, cloned, title || '', String(url));
            }
            histStack[histIdx] = {
                state: cloned === undefined ? null : cloned,
                title: String(title || ''),
                url: (globalObj.location && globalObj.location.href) || String(url || '')
            };
        };
        globalObj.history.go = function(delta) {
            var d = delta | 0;
            var next = histIdx + d;
            if (next >= 0 && next < histStack.length) {
                histIdx = next;
                var entry = histStack[histIdx];
                if (entry.url && typeof nativePush === 'function') {
                    nativePush.call(globalObj.history, entry.state, entry.title, entry.url);
                }
                var popEv = new PopStateEvent('popstate', { state: entry.state, bubbles: false });
                globalObj.dispatchEvent(popEv);
            }
        };
        globalObj.history.back = function() {
            globalObj.history.go(-1);
        };
        globalObj.history.forward = function() {
            globalObj.history.go(1);
        };
    }

    // =========================================================================
    // 8. 2D Canvas Context with Transforms, Paths, Gradients & ImageData (Pillar 4B)
    // =========================================================================
    class CanvasGradient {
        constructor(kind, coords) {
            this._isGradient = true;
            this.kind = kind;
            this.coords = coords;
            this.stops = [];
        }
        addColorStop(offset, color) {
            this.stops.push({ offset: Number(offset), color: String(color || '#000000') });
            this.stops.sort(function(a, b) { return a.offset - b.offset; });
        }
    }

    if (elProto && typeof elProto.getContext === 'function') {
        var nativeGetContext = elProto.getContext;
        elProto.getContext = function(contextType) {
            if (String(contextType || '2d').toLowerCase() !== '2d') {
                return null;
            }
            if (this._ctx2d) return this._ctx2d;
            var raw = nativeGetContext.call(this, '2d');
            if (!raw) return null;

            var stateStack = [];
            var matrix = [1, 0, 0, 1, 0, 0]; // [a, b, c, d, e, f]
            var lineDash = [];
            var subpaths = [];
            var curSubpath = [];

            function txPoint(x, y) {
                var nx = matrix[0] * x + matrix[2] * y + matrix[4];
                var ny = matrix[1] * x + matrix[3] * y + matrix[5];
                return [Math.round(nx), Math.round(ny)];
            }

            function resolveStyleString(styleVal) {
                if (styleVal && styleVal._isGradient && styleVal.stops.length > 0) {
                    return styleVal.stops[0].color;
                }
                return String(styleVal || '#000000');
            }

            var ctx = {
                canvas: this,
                fillStyle: '#000000',
                strokeStyle: '#000000',
                lineWidth: 1,
                globalAlpha: 1,
                font: '10px sans-serif',
                textAlign: 'start',
                textBaseline: 'alphabetic',
                lineCap: 'butt',
                lineJoin: 'miter',
                lineDashOffset: 0,

                save: function() {
                    stateStack.push({
                        fillStyle: this.fillStyle,
                        strokeStyle: this.strokeStyle,
                        lineWidth: this.lineWidth,
                        globalAlpha: this.globalAlpha,
                        font: this.font,
                        textAlign: this.textAlign,
                        textBaseline: this.textBaseline,
                        lineCap: this.lineCap,
                        lineJoin: this.lineJoin,
                        matrix: matrix.slice(),
                        lineDash: lineDash.slice()
                    });
                },
                restore: function() {
                    if (stateStack.length > 0) {
                        var s = stateStack.pop();
                        this.fillStyle = s.fillStyle;
                        this.strokeStyle = s.strokeStyle;
                        this.lineWidth = s.lineWidth;
                        this.globalAlpha = s.globalAlpha;
                        this.font = s.font;
                        this.textAlign = s.textAlign;
                        this.textBaseline = s.textBaseline;
                        this.lineCap = s.lineCap;
                        this.lineJoin = s.lineJoin;
                        matrix = s.matrix;
                        lineDash = s.lineDash;
                    }
                },
                translate: function(tx, ty) {
                    var x = Number(tx) || 0;
                    var y = Number(ty) || 0;
                    matrix[4] += matrix[0] * x + matrix[2] * y;
                    matrix[5] += matrix[1] * x + matrix[3] * y;
                },
                scale: function(sx, sy) {
                    var x = Number(sx) || 1;
                    var y = (sy === undefined) ? x : (Number(sy) || 1);
                    matrix[0] *= x;
                    matrix[1] *= x;
                    matrix[2] *= y;
                    matrix[3] *= y;
                },
                rotate: function(angle) {
                    var rad = Number(angle) || 0;
                    var c = Math.cos(rad);
                    var s = Math.sin(rad);
                    var a0 = matrix[0], b0 = matrix[1], c0 = matrix[2], d0 = matrix[3];
                    matrix[0] = a0 * c + c0 * s;
                    matrix[1] = b0 * c + d0 * s;
                    matrix[2] = a0 * (-s) + c0 * c;
                    matrix[3] = b0 * (-s) + d0 * c;
                },
                transform: function(a, b, c, d, e, f) {
                    var a0 = matrix[0], b0 = matrix[1], c0 = matrix[2], d0 = matrix[3], e0 = matrix[4], f0 = matrix[5];
                    matrix[0] = a0 * a + c0 * b;
                    matrix[1] = b0 * a + d0 * b;
                    matrix[2] = a0 * c + c0 * d;
                    matrix[3] = b0 * c + d0 * d;
                    matrix[4] = a0 * e + c0 * f + e0;
                    matrix[5] = b0 * e + d0 * f + f0;
                },
                setTransform: function(a, b, c, d, e, f) {
                    if (arguments.length === 0) {
                        matrix = [1, 0, 0, 1, 0, 0];
                    } else {
                        matrix = [Number(a) || 0, Number(b) || 0, Number(c) || 0, Number(d) || 0, Number(e) || 0, Number(f) || 0];
                    }
                },
                resetTransform: function() {
                    matrix = [1, 0, 0, 1, 0, 0];
                },
                getTransform: function() {
                    return { a: matrix[0], b: matrix[1], c: matrix[2], d: matrix[3], e: matrix[4], f: matrix[5] };
                },
                setLineDash: function(segments) {
                    lineDash = Array.isArray(segments) ? segments.slice() : [];
                },
                getLineDash: function() {
                    return lineDash.slice();
                },
                createLinearGradient: function(x0, y0, x1, y1) {
                    return new CanvasGradient('linear', [Number(x0) || 0, Number(y0) || 0, Number(x1) || 0, Number(y1) || 0]);
                },
                createRadialGradient: function(x0, y0, r0, x1, y1, r1) {
                    return new CanvasGradient('radial', [Number(x0) || 0, Number(y0) || 0, Number(r0) || 0, Number(x1) || 0, Number(y1) || 0, Number(r1) || 0]);
                },
                fillRect: function(x, y, w, h) {
                    var p0 = txPoint(x, y);
                    var p1 = txPoint(x + w, y + h);
                    var rx = Math.min(p0[0], p1[0]);
                    var ry = Math.min(p0[1], p1[1]);
                    var rw = Math.max(1, Math.abs(p1[0] - p0[0]));
                    var rh = Math.max(1, Math.abs(p1[1] - p0[1]));
                    if (this.fillStyle && this.fillStyle._isGradient && typeof raw.__fillLinearGradient === 'function') {
                        var g = this.fillStyle;
                        var c0 = g.stops.length > 0 ? g.stops[0].color : '#000000';
                        var c1 = g.stops.length > 1 ? g.stops[g.stops.length - 1].color : c0;
                        var horiz = Math.abs(g.coords[2] - g.coords[0]) >= Math.abs(g.coords[3] - g.coords[1]);
                        raw.__fillLinearGradient(rx, ry, rw, rh, c0, c1, horiz);
                        return;
                    }
                    raw.fillStyle = resolveStyleString(this.fillStyle);
                    raw.fillRect(rx, ry, rw, rh);
                },
                strokeRect: function(x, y, w, h) {
                    var p0 = txPoint(x, y);
                    var p1 = txPoint(x + w, y + h);
                    raw.strokeStyle = resolveStyleString(this.strokeStyle);
                    raw.lineWidth = this.lineWidth;
                    raw.strokeRect(
                        Math.min(p0[0], p1[0]),
                        Math.min(p0[1], p1[1]),
                        Math.max(1, Math.abs(p1[0] - p0[0])),
                        Math.max(1, Math.abs(p1[1] - p0[1]))
                    );
                },
                clearRect: function(x, y, w, h) {
                    var p0 = txPoint(x, y);
                    var p1 = txPoint(x + w, y + h);
                    raw.clearRect(
                        Math.min(p0[0], p1[0]),
                        Math.min(p0[1], p1[1]),
                        Math.max(1, Math.abs(p1[0] - p0[0])),
                        Math.max(1, Math.abs(p1[1] - p0[1]))
                    );
                },
                fillText: function(text, x, y) {
                    var p = txPoint(x, y);
                    raw.fillStyle = resolveStyleString(this.fillStyle);
                    raw.fillText(String(text || ''), p[0], p[1]);
                },
                strokeText: function(text, x, y) {
                    var p = txPoint(x, y);
                    raw.fillStyle = resolveStyleString(this.strokeStyle);
                    raw.fillText(String(text || ''), p[0], p[1]);
                },
                measureText: function(text) {
                    return raw.measureText(String(text || ''));
                },
                beginPath: function() {
                    subpaths = [];
                    curSubpath = [];
                    raw.beginPath();
                },
                moveTo: function(x, y) {
                    if (curSubpath.length > 0) subpaths.push(curSubpath);
                    var p = txPoint(x, y);
                    curSubpath = [p];
                    raw.moveTo(p[0], p[1]);
                },
                lineTo: function(x, y) {
                    var p = txPoint(x, y);
                    curSubpath.push(p);
                    raw.lineTo(p[0], p[1]);
                },
                quadraticCurveTo: function(cpx, cpy, x, y) {
                    var start = curSubpath.length > 0 ? curSubpath[curSubpath.length - 1] : txPoint(0, 0);
                    var cp = txPoint(cpx, cpy);
                    var end = txPoint(x, y);
                    for (var i = 1; i <= 12; i++) {
                        var t = i / 12;
                        var inv = 1 - t;
                        var px = Math.round(inv * inv * start[0] + 2 * inv * t * cp[0] + t * t * end[0]);
                        var py = Math.round(inv * inv * start[1] + 2 * inv * t * cp[1] + t * t * end[1]);
                        curSubpath.push([px, py]);
                        raw.lineTo(px, py);
                    }
                },
                bezierCurveTo: function(cp1x, cp1y, cp2x, cp2y, x, y) {
                    var start = curSubpath.length > 0 ? curSubpath[curSubpath.length - 1] : txPoint(0, 0);
                    var cp1 = txPoint(cp1x, cp1y);
                    var cp2 = txPoint(cp2x, cp2y);
                    var end = txPoint(x, y);
                    for (var i = 1; i <= 16; i++) {
                        var t = i / 16;
                        var inv = 1 - t;
                        var px = Math.round(
                            inv * inv * inv * start[0] +
                            3 * inv * inv * t * cp1[0] +
                            3 * inv * t * t * cp2[0] +
                            t * t * t * end[0]
                        );
                        var py = Math.round(
                            inv * inv * inv * start[1] +
                            3 * inv * inv * t * cp1[1] +
                            3 * inv * t * t * cp2[1] +
                            t * t * t * end[1]
                        );
                        curSubpath.push([px, py]);
                        raw.lineTo(px, py);
                    }
                },
                rect: function(x, y, w, h) {
                    var p0 = txPoint(x, y);
                    var p1 = txPoint(x + w, y);
                    var p2 = txPoint(x + w, y + h);
                    var p3 = txPoint(x, y + h);
                    if (curSubpath.length > 0) subpaths.push(curSubpath);
                    curSubpath = [p0, p1, p2, p3, p0];
                    raw.rect(p0[0], p0[1], p2[0] - p0[0], p2[1] - p0[1]);
                },
                roundRect: function(x, y, w, h) {
                    this.rect(x, y, w, h);
                },
                arc: function(x, y, r, startAngle, endAngle) {
                    var a0 = Number(startAngle) || 0;
                    var a1 = (endAngle === undefined) ? (Math.PI * 2) : Number(endAngle);
                    var steps = 20;
                    for (var i = 0; i <= steps; i++) {
                        var t = a0 + (a1 - a0) * (i / steps);
                        var p = txPoint(x + r * Math.cos(t), y + r * Math.sin(t));
                        curSubpath.push(p);
                        raw.lineTo(p[0], p[1]);
                    }
                },
                ellipse: function(x, y, rx, ry, rot, startAngle, endAngle) {
                    var a0 = Number(startAngle) || 0;
                    var a1 = (endAngle === undefined) ? (Math.PI * 2) : Number(endAngle);
                    var steps = 20;
                    for (var i = 0; i <= steps; i++) {
                        var t = a0 + (a1 - a0) * (i / steps);
                        var p = txPoint(x + rx * Math.cos(t), y + ry * Math.sin(t));
                        curSubpath.push(p);
                        raw.lineTo(p[0], p[1]);
                    }
                },
                closePath: function() {
                    if (curSubpath.length > 0) {
                        curSubpath.push(curSubpath[0]);
                    }
                    raw.closePath();
                },
                fill: function() {
                    raw.fillStyle = resolveStyleString(this.fillStyle);
                    raw.fill();
                },
                stroke: function() {
                    raw.strokeStyle = resolveStyleString(this.strokeStyle);
                    raw.lineWidth = this.lineWidth;
                    raw.stroke();
                },
                clip: function() {},
                getImageData: function(sx, sy, sw, sh) {
                    return raw.getImageData(sx, sy, sw, sh);
                },
                putImageData: function(imgData, dx, dy) {
                    return raw.putImageData(imgData, dx, dy);
                },
                createImageData: function(w, h) {
                    return raw.createImageData(w, h);
                }
            };
            this._ctx2d = ctx;
            return ctx;
        };
    }

    // =========================================================================
    // 9. AdoptedStyleSheets, CSSStyleSheet, DOMParser, Observers, URL, Blob
    // =========================================================================
    Object.defineProperty(document, 'adoptedStyleSheets', {
        enumerable: true,
        configurable: true,
        get: function() { return this._adoptedSheets || []; },
        set: function(arr) {
            var list = Array.isArray(arr) ? arr : [];
            this._adoptedSheets = list;
            var texts = [];
            for (var i = 0; i < list.length; i++) {
                if (list[i] && typeof list[i]._cssText === 'string') {
                    texts.push(list[i]._cssText);
                }
            }
            document.__setAdoptedSheets(texts.join('\n'));
        }
    });

    class CSSStyleSheet {
        constructor() {
            this._cssText = '';
            this.cssRules = [];
        }
        replaceSync(text) {
            this._cssText = String(text || '');
            this.cssRules = this._cssText ? [{ cssText: this._cssText }] : [];
            var sheets = document._adoptedSheets || [];
            if (sheets.indexOf(this) !== -1) {
                document.adoptedStyleSheets = sheets;
            }
        }
        replace(text) {
            this.replaceSync(text);
            return Promise.resolve(this);
        }
        insertRule(rule, index) {
            var r = String(rule || '');
            this._cssText = this._cssText ? (this._cssText + '\n' + r) : r;
            var idx = (typeof index === 'number') ? index : this.cssRules.length;
            this.cssRules.push({ cssText: r });
            var sheets = document._adoptedSheets || [];
            if (sheets.indexOf(this) !== -1) {
                document.adoptedStyleSheets = sheets;
            }
            return idx;
        }
        deleteRule(index) {
            if (typeof index === 'number' && index >= 0 && index < this.cssRules.length) {
                this.cssRules.splice(index, 1);
            }
        }
    }

    class DOMParser {
        parseFromString(markup) {
            return document.__parseHTML(String(markup || ''));
        }
    }

    class MutationObserver {
        constructor(callback) {
            this._cb = callback;
            this._records = [];
            var self = this;
            this._notify = function(rec) {
                self._records.push(rec);
                if (!self._scheduled) {
                    self._scheduled = true;
                    queueMicrotask(function() {
                        self._scheduled = false;
                        if (self._records.length > 0 && typeof self._cb === 'function') {
                            var batch = self._records.slice();
                            self._records = [];
                            self._cb(batch, self);
                        }
                    });
                }
            };
        }
        observe(target) {
            if (target) {
                globalObj.__registerMutationObserver(this._notify, target);
            }
        }
        disconnect() {
            globalObj.__unregisterMutationObserver(this._notify);
            this._records = [];
        }
        takeRecords() {
            var out = this._records.slice();
            this._records = [];
            return out;
        }
    }

    class IntersectionObserver {
        constructor(callback, options) {
            this._cb = callback;
            this.root = (options && options.root) || null;
            this.rootMargin = (options && options.rootMargin) || '0px';
            this.thresholds = [0];
            this._entries = [];
        }
        observe(target) {
            if (!target) return;
            var rect = typeof target.getBoundingClientRect === 'function'
                ? target.getBoundingClientRect()
                : { x: 0, y: 0, top: 0, left: 0, width: 100, height: 24, right: 100, bottom: 24 };
            var entry = {
                target: target,
                isIntersecting: true,
                intersectionRatio: 1,
                boundingClientRect: rect,
                intersectionRect: rect,
                rootBounds: rect,
                time: 0
            };
            var cb = this._cb;
            var self = this;
            queueMicrotask(function() {
                if (typeof cb === 'function') cb([entry], self);
            });
        }
        unobserve() {}
        disconnect() {}
        takeRecords() { return []; }
    }

    class ResizeObserver {
        constructor(callback) {
            this._cb = callback;
        }
        observe(target) {
            if (!target) return;
            var rect = typeof target.getBoundingClientRect === 'function'
                ? target.getBoundingClientRect()
                : { x: 0, y: 0, top: 0, left: 0, width: 100, height: 24, right: 100, bottom: 24 };
            var entry = {
                target: target,
                contentRect: rect,
                borderBoxSize: [{ inlineSize: rect.width, blockSize: rect.height }],
                contentBoxSize: [{ inlineSize: rect.width, blockSize: rect.height }]
            };
            var cb = this._cb;
            var self = this;
            queueMicrotask(function() {
                if (typeof cb === 'function') cb([entry], self);
            });
        }
        unobserve() {}
        disconnect() {}
    }

    class URLSearchParams {
        constructor(init) {
            this._pairs = [];
            if (!init) return;
            if (typeof init === 'string') {
                var s = init.charAt(0) === '?' ? init.slice(1) : init;
                if (!s) return;
                var parts = s.split('&');
                for (var i = 0; i < parts.length; i++) {
                    if (!parts[i]) continue;
                    var eq = parts[i].indexOf('=');
                    if (eq === -1) {
                        this._pairs.push([decodeURIComponent(parts[i].replace(/\+/g, ' ')), '']);
                    } else {
                        var k = decodeURIComponent(parts[i].slice(0, eq).replace(/\+/g, ' '));
                        var v = decodeURIComponent(parts[i].slice(eq + 1).replace(/\+/g, ' '));
                        this._pairs.push([k, v]);
                    }
                }
            } else if (Array.isArray(init)) {
                for (var j = 0; j < init.length; j++) {
                    if (init[j] && init[j].length >= 2) {
                        this._pairs.push([String(init[j][0]), String(init[j][1])]);
                    }
                }
            } else if (typeof init === 'object') {
                var keys = Object.keys(init);
                for (var m = 0; m < keys.length; m++) {
                    this._pairs.push([keys[m], String(init[keys[m]])]);
                }
            }
        }
        append(name, value) {
            this._pairs.push([String(name), String(value)]);
            if (this._onChange) this._onChange();
        }
        set(name, value) {
            var k = String(name);
            var v = String(value);
            var found = false;
            var next = [];
            for (var i = 0; i < this._pairs.length; i++) {
                if (this._pairs[i][0] === k) {
                    if (!found) {
                        next.push([k, v]);
                        found = true;
                    }
                } else {
                    next.push(this._pairs[i]);
                }
            }
            if (!found) next.push([k, v]);
            this._pairs = next;
            if (this._onChange) this._onChange();
        }
        get(name) {
            var k = String(name);
            for (var i = 0; i < this._pairs.length; i++) {
                if (this._pairs[i][0] === k) return this._pairs[i][1];
            }
            return null;
        }
        getAll(name) {
            var k = String(name);
            var out = [];
            for (var i = 0; i < this._pairs.length; i++) {
                if (this._pairs[i][0] === k) out.push(this._pairs[i][1]);
            }
            return out;
        }
        has(name) {
            return this.get(name) !== null;
        }
        delete(name) {
            var k = String(name);
            var next = [];
            for (var i = 0; i < this._pairs.length; i++) {
                if (this._pairs[i][0] !== k) next.push(this._pairs[i]);
            }
            this._pairs = next;
            if (this._onChange) this._onChange();
        }
        toString() {
            var out = [];
            for (var i = 0; i < this._pairs.length; i++) {
                out.push(encodeURIComponent(this._pairs[i][0]) + '=' + encodeURIComponent(this._pairs[i][1]));
            }
            return out.join('&');
        }
    }

    class URL {
        constructor(input, base) {
            var raw = String(input || '');
            if (raw.indexOf('://') === -1 && base) {
                var b = String(base);
                if (raw.charAt(0) === '/') {
                    var m = b.match(/^([a-zA-Z]+:\/\/[^\/]+)/);
                    raw = (m ? m[1] : 'https://localhost') + raw;
                } else {
                    var lastSlash = b.lastIndexOf('/');
                    raw = (lastSlash >= 0 ? b.slice(0, lastSlash + 1) : b + '/') + raw;
                }
            }
            var hashIdx = raw.indexOf('#');
            this.hash = hashIdx >= 0 ? raw.slice(hashIdx) : '';
            var beforeHash = hashIdx >= 0 ? raw.slice(0, hashIdx) : raw;
            var qIdx = beforeHash.indexOf('?');
            var rawSearch = qIdx >= 0 ? beforeHash.slice(qIdx) : '';
            var beforeQ = qIdx >= 0 ? beforeHash.slice(0, qIdx) : beforeHash;
            var schemeSep = beforeQ.indexOf('://');
            this.protocol = schemeSep >= 0 ? beforeQ.slice(0, schemeSep + 1) : 'https:';
            var afterScheme = schemeSep >= 0 ? beforeQ.slice(schemeSep + 3) : beforeQ;
            var slashIdx = afterScheme.indexOf('/');
            this.host = slashIdx >= 0 ? afterScheme.slice(0, slashIdx) : afterScheme;
            var colonIdx = this.host.indexOf(':');
            this.hostname = colonIdx >= 0 ? this.host.slice(0, colonIdx) : this.host;
            this.port = colonIdx >= 0 ? this.host.slice(colonIdx + 1) : '';
            this.pathname = slashIdx >= 0 ? afterScheme.slice(slashIdx) : '/';
            this.origin = this.protocol + '//' + this.host;
            this.searchParams = new URLSearchParams(rawSearch);
            var self = this;
            this.searchParams._onChange = function() {
                var s = self.searchParams.toString();
                self.search = s ? ('?' + s) : '';
                self.href = self.origin + self.pathname + self.search + self.hash;
            };
            this.searchParams._onChange();
        }
        toString() {
            return this.href;
        }
        toJSON() {
            return this.href;
        }
    }

    class FormData {
        constructor() {
            this._entries = [];
        }
        append(name, value) {
            this._entries.push([String(name), String(value)]);
        }
        set(name, value) {
            var k = String(name);
            var v = String(value);
            var next = [];
            var placed = false;
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i][0] === k) {
                    if (!placed) {
                        next.push([k, v]);
                        placed = true;
                    }
                } else {
                    next.push(this._entries[i]);
                }
            }
            if (!placed) next.push([k, v]);
            this._entries = next;
        }
        get(name) {
            var k = String(name);
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i][0] === k) return this._entries[i][1];
            }
            return null;
        }
        has(name) {
            return this.get(name) !== null;
        }
        delete(name) {
            var k = String(name);
            var next = [];
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i][0] !== k) next.push(this._entries[i]);
            }
            this._entries = next;
        }
    }

    class Blob {
        constructor(parts, options) {
            var list = Array.isArray(parts) ? parts : [];
            var acc = '';
            for (var i = 0; i < list.length; i++) {
                acc += String(list[i]);
            }
            this._text = acc;
            this.size = acc.length;
            this.type = (options && options.type) ? String(options.type) : '';
        }
        text() {
            return Promise.resolve(this._text);
        }
    }

    class TextEncoder {
        constructor() {
            this.encoding = 'utf-8';
        }
        encode(input) {
            var s = String(input || '');
            var out = [];
            for (var i = 0; i < s.length; i++) {
                out.push(s.charCodeAt(i) & 0xFF);
            }
            return out;
        }
    }

    class TextDecoder {
        constructor(label) {
            this.encoding = String(label || 'utf-8').toLowerCase();
        }
        decode(bytes) {
            if (!bytes) return '';
            var out = '';
            for (var i = 0; i < bytes.length; i++) {
                out += String.fromCharCode(bytes[i]);
            }
            return out;
        }
    }

    class MediaSource extends EventTarget {
        constructor() {
            super();
            this.readyState = 'open';
            this.sourceBuffers = [];
            this.activeSourceBuffers = [];
            this.duration = 0;
        }
        addSourceBuffer(mime) {
            var sb = new EventTarget();
            sb.mode = 'segments';
            sb.updating = false;
            sb.mimeType = String(mime || 'video/mp4');
            sb.appendBuffer = function(_buf) {
                sb.updating = false;
                sb.dispatchEvent(new Event('updateend'));
            };
            this.sourceBuffers.push(sb);
            this.activeSourceBuffers.push(sb);
            return sb;
        }
        endOfStream() {
            this.readyState = 'ended';
        }
        static isTypeSupported(mime) {
            var m = String(mime || '').toLowerCase();
            return m.indexOf('video/mp4') !== -1 || m.indexOf('audio/mp4') !== -1 || m.indexOf('video/webm') !== -1 || m.indexOf('audio/webm') !== -1;
        }
    }

    globalObj.EventTarget = EventTarget;
    globalObj.MediaSource = MediaSource;
    globalObj.Event = Event;
    globalObj.CustomEvent = CustomEvent;
    globalObj.UIEvent = UIEvent;
    globalObj.MouseEvent = MouseEvent;
    globalObj.KeyboardEvent = KeyboardEvent;
    globalObj.InputEvent = InputEvent;
    globalObj.FocusEvent = FocusEvent;
    globalObj.ErrorEvent = ErrorEvent;
    globalObj.PromiseRejectionEvent = PromiseRejectionEvent;
    globalObj.PopStateEvent = PopStateEvent;
    globalObj.AbortController = AbortController;
    globalObj.AbortSignal = AbortSignal;
    globalObj.Headers = Headers;
    globalObj.Request = Request;
    globalObj.Response = Response;
    globalObj.DataView = DataView;
    globalObj.NodeFilter = NodeFilter;
    globalObj.TreeWalker = TreeWalker;
    globalObj.NodeIterator = NodeIterator;
    globalObj.Range = Range;
    globalObj.CanvasGradient = CanvasGradient;
    globalObj.CSSStyleSheet = CSSStyleSheet;
    globalObj.DOMParser = DOMParser;
    globalObj.MutationObserver = MutationObserver;
    globalObj.IntersectionObserver = IntersectionObserver;
    globalObj.ResizeObserver = ResizeObserver;
    globalObj.URLSearchParams = URLSearchParams;
    globalObj.URL = URL;
    globalObj.FormData = FormData;
    globalObj.Blob = Blob;
    globalObj.TextEncoder = TextEncoder;
    globalObj.TextDecoder = TextDecoder;
})();
