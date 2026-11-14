// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// Standard WHATWG DOM, 3-Phase Event Propagation, Fetch/Abort, Traversal,
// Web Crypto, Performance, History, DataView, and 2D Canvas Runtime for Cinder.

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
        if (typeof target.__nativeAddListener === 'function') {
            var accepted = true;
            try {
                accepted = target.__nativeAddListener(lower, function() {}) !== false;
            } catch (_e) {
                accepted = false;
            }
            if (!accepted) {
                list.pop();
                return;
            }
        }
        if (opts.signal && typeof opts.signal.addEventListener === 'function') {
            opts.signal.addEventListener('abort', function() {
                removeListenerFromTarget(target, evType, callback, opts.capture);
            }, { once: true });
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
            var prevEvent = globalObj.event;
            eventObj._passive = Boolean(entry.passive);
            globalObj.event = eventObj;
            invokedAny = true;
            try {
                var ret;
                if (typeof entry.callback === 'function') {
                    ret = entry.callback.call(currentTarget, eventObj);
                } else if (entry.callback && typeof entry.callback.handleEvent === 'function') {
                    ret = entry.callback.handleEvent.call(entry.callback, eventObj);
                }
                // addEventListener ignores the return value. Only the inline
                // handler below treats false as cancel.
            } catch (err) {
                dispatchGlobalError(err && err.message ? err.message : String(err), '', 1, 1, err);
            }
            globalObj.event = prevEvent;
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
    globalObj.__dispatchPageTransition = function(type, persisted) {
        var ev = {
            type: String(type || ''),
            persisted: Boolean(persisted),
            bubbles: false,
            cancelable: false,
            defaultPrevented: false,
            target: (typeof document !== 'undefined' && document.body) ? document.body : globalObj,
            currentTarget: globalObj,
            _stopped: false,
            _immediateStopped: false,
            _passive: false,
            preventDefault: function() { this.defaultPrevented = true; }
        };
        return invokeListenersAt(globalObj, ev, 2);
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
        elProto.click = function() {
            var ev = new MouseEvent('click', { bubbles: true, cancelable: true, isTrusted: true });
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
        } else if (lower === 'invalid') {
            ev = new Event('invalid', { bubbles: false, cancelable: true, isTrusted: true });
        } else if (lower === 'close') {
            ev = new Event('close', { bubbles: false, cancelable: false, isTrusted: true });
        } else if (lower === 'submit') {
            ev = new Event('submit', { bubbles: true, cancelable: true, isTrusted: true });
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

    function serializeBodyPayload(rawBody) {
        if (rawBody === undefined || rawBody === null) {
            return { body: '', contentType: '' };
        }
        if (typeof rawBody === 'object') {
            if (typeof rawBody.toMultipart === 'function') {
                var bnd = rawBody._boundary || '----AnvilFormBoundary7MA4YWxkTrZu0gW';
                return {
                    body: rawBody.toMultipart(bnd),
                    contentType: 'multipart/form-data; boundary=' + bnd
                };
            }
            if (Array.isArray(rawBody._pairs) && typeof rawBody.getAll === 'function') {
                return {
                    body: rawBody.toString(),
                    contentType: 'application/x-www-form-urlencoded;charset=UTF-8'
                };
            }
            if (typeof rawBody._text === 'string') {
                return {
                    body: rawBody._text,
                    contentType: rawBody.type || 'application/octet-stream'
                };
            }
        }
        return { body: String(rawBody), contentType: '' };
    }

    class Request {
        constructor(input, init) {
            if (input instanceof Request) {
                this.url = input.url;
                this.method = (init && init.method) ? String(init.method).toUpperCase() : input.method;
                this.headers = new Headers((init && init.headers) || input.headers);
                if (init && 'body' in init) {
                    var sb0 = serializeBodyPayload(init.body);
                    this._body = sb0.body;
                    if (sb0.contentType && !this.headers.has('content-type')) {
                        this.headers.set('content-type', sb0.contentType);
                    }
                } else {
                    this._body = input._body;
                }
                this.signal = (init && init.signal) || input.signal || new AbortSignal();
                this.credentials = (init && init.credentials) ? String(init.credentials) : (input.credentials || 'same-origin');
                this.mode = (init && init.mode) ? String(init.mode) : (input.mode || 'cors');
                this._corsOptIn = Boolean(
                    (init && (init.mode === 'cors' || init.credentials === 'include' || init.headers || (init.method && String(init.method).toUpperCase() !== 'GET' && String(init.method).toUpperCase() !== 'HEAD')))
                    || input._corsOptIn
                );
            } else {
                this.url = String(input || '');
                this.method = (init && init.method) ? String(init.method).toUpperCase() : 'GET';
                this.headers = new Headers(init && init.headers);
                if (init && init.body !== undefined && init.body !== null) {
                    var sb1 = serializeBodyPayload(init.body);
                    this._body = sb1.body;
                    if (sb1.contentType && !this.headers.has('content-type')) {
                        this.headers.set('content-type', sb1.contentType);
                    }
                } else {
                    this._body = '';
                }
                this.signal = (init && init.signal) || new AbortSignal();
                this.credentials = (init && init.credentials) ? String(init.credentials) : 'same-origin';
                this.mode = (init && init.mode) ? String(init.mode) : 'cors';
                this._corsOptIn = Boolean(
                    init && (init.mode === 'cors' || init.credentials === 'include' || init.headers || (init.method && String(init.method).toUpperCase() !== 'GET' && String(init.method).toUpperCase() !== 'HEAD'))
                );
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

    function utf8EncodeToBytes(input) {
        var s = String(input === undefined || input === null ? '' : input);
        var out = [];
        for (var i = 0; i < s.length; i++) {
            var c = s.charCodeAt(i);
            if (c < 0x80) {
                out.push(c);
            } else if (c < 0x800) {
                out.push(0xC0 | (c >> 6));
                out.push(0x80 | (c & 0x3F));
            } else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < s.length) {
                var c2 = s.charCodeAt(i + 1);
                if (c2 >= 0xDC00 && c2 <= 0xDFFF) {
                    var cp = ((c - 0xD800) << 10) + (c2 - 0xDC00) + 0x10000;
                    out.push(0xF0 | (cp >> 18));
                    out.push(0x80 | ((cp >> 12) & 0x3F));
                    out.push(0x80 | ((cp >> 6) & 0x3F));
                    out.push(0x80 | (cp & 0x3F));
                    i++;
                } else {
                    out.push(0xE0 | (c >> 12));
                    out.push(0x80 | ((c >> 6) & 0x3F));
                    out.push(0x80 | (c & 0x3F));
                }
            } else {
                out.push(0xE0 | (c >> 12));
                out.push(0x80 | ((c >> 6) & 0x3F));
                out.push(0x80 | (c & 0x3F));
            }
        }
        return out;
    }

    class ReadableStreamDefaultReader {
        constructor(stream) {
            this._stream = stream;
            this.closed = Promise.resolve(undefined);
        }
        read() {
            if (!this._stream || this._stream._queue.length === 0) {
                return Promise.resolve({ value: undefined, done: true });
            }
            var chunk = this._stream._queue.shift();
            return Promise.resolve({ value: chunk, done: false });
        }
        releaseLock() {
            if (this._stream) this._stream.locked = false;
        }
        cancel(reason) {
            if (this._stream) return this._stream.cancel(reason);
            return Promise.resolve(undefined);
        }
    }

    class ReadableStream {
        constructor(underlyingSource) {
            this.locked = false;
            this._queue = [];
            this._closed = false;
            var self = this;
            var controller = {
                desiredSize: 1,
                enqueue: function(chunk) {
                    if (!self._closed) self._queue.push(chunk);
                },
                close: function() {
                    self._closed = true;
                },
                error: function(_e) {
                    self._closed = true;
                }
            };
            this._controller = controller;
            if (underlyingSource && typeof underlyingSource.start === 'function') {
                try { underlyingSource.start(controller); } catch (_) {}
            }
            if (underlyingSource && typeof underlyingSource.pull === 'function') {
                try { underlyingSource.pull(controller); } catch (_) {}
            }
        }
        getReader() {
            this.locked = true;
            return new ReadableStreamDefaultReader(this);
        }
        cancel(_reason) {
            this._queue = [];
            this._closed = true;
            this.locked = false;
            return Promise.resolve(undefined);
        }
        tee() {
            var s1 = new ReadableStream();
            var s2 = new ReadableStream();
            s1._queue = this._queue.slice();
            s2._queue = this._queue.slice();
            return [s1, s2];
        }
        pipeThrough(transform) {
            if (transform && transform.writable && transform.readable) {
                var w = transform.writable.getWriter();
                while (this._queue.length > 0) {
                    w.write(this._queue.shift());
                }
                w.close();
                return transform.readable;
            }
            return this;
        }
        pipeTo(dest) {
            if (dest && typeof dest.getWriter === 'function') {
                var w = dest.getWriter();
                while (this._queue.length > 0) {
                    w.write(this._queue.shift());
                }
                w.close();
            }
            return Promise.resolve(undefined);
        }
    }

    class WritableStream {
        constructor(underlyingSink) {
            this.locked = false;
            this._sink = underlyingSink || {};
            this._chunks = [];
            if (this._sink && typeof this._sink.start === 'function') {
                try { this._sink.start({}); } catch (_) {}
            }
        }
        getWriter() {
            this.locked = true;
            var self = this;
            return {
                ready: Promise.resolve(undefined),
                closed: Promise.resolve(undefined),
                desiredSize: 1,
                write: function(chunk) {
                    self._chunks.push(chunk);
                    if (self._sink && typeof self._sink.write === 'function') {
                        try { self._sink.write(chunk, {}); } catch (_) {}
                    }
                    return Promise.resolve(undefined);
                },
                close: function() {
                    self.locked = false;
                    if (self._sink && typeof self._sink.close === 'function') {
                        try { self._sink.close(); } catch (_) {}
                    }
                    return Promise.resolve(undefined);
                },
                abort: function(reason) {
                    self.locked = false;
                    if (self._sink && typeof self._sink.abort === 'function') {
                        try { self._sink.abort(reason); } catch (_) {}
                    }
                    return Promise.resolve(undefined);
                },
                releaseLock: function() {
                    self.locked = false;
                }
            };
        }
        close() {
            return Promise.resolve(undefined);
        }
        abort(_reason) {
            return Promise.resolve(undefined);
        }
    }

    class TransformStream {
        constructor(transformer) {
            var readable = new ReadableStream();
            var tx = transformer || {};
            var ctrl = {
                desiredSize: 1,
                enqueue: function(chunk) {
                    readable._controller.enqueue(chunk);
                },
                terminate: function() {
                    readable._controller.close();
                },
                error: function(e) {
                    readable._controller.error(e);
                }
            };
            if (typeof tx.start === 'function') {
                try { tx.start(ctrl); } catch (_) {}
            }
            var writable = new WritableStream({
                write: function(chunk) {
                    if (typeof tx.transform === 'function') {
                        tx.transform(chunk, ctrl);
                    } else {
                        ctrl.enqueue(chunk);
                    }
                },
                close: function() {
                    if (typeof tx.flush === 'function') {
                        try { tx.flush(ctrl); } catch (_) {}
                    }
                    readable._controller.close();
                }
            });
            this.readable = readable;
            this.writable = writable;
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
            var rawText = this._body;
            this.body = new ReadableStream({
                start: function(c) {
                    if (rawText.length > 0) {
                        var encoded = utf8EncodeToBytes(rawText);
                        var ab = new globalObj.ArrayBuffer(encoded.length);
                        var u8 = new globalObj.Uint8Array(ab);
                        for (var i = 0; i < encoded.length; i++) u8[i] = encoded[i];
                        c.enqueue(u8);
                    }
                    c.close();
                }
            });
        }
        text() {
            this.bodyUsed = true;
            return Promise.resolve(this._body);
        }
        json() {
            this.bodyUsed = true;
            var raw = this._body;
            var parsed;
            try {
                parsed = JSON.parse(raw);
            } catch (err) {
                return Promise.reject(err);
            }
            return parsed;
        }
        arrayBuffer() {
            this.bodyUsed = true;
            var bytes = utf8EncodeToBytes(this._body);
            var buf = new ArrayBuffer(bytes.length);
            var view = new Uint8Array(buf);
            for (var i = 0; i < bytes.length; i++) {
                view[i] = bytes[i];
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

    function settledResponse(body, status, url, headersInit) {
        var response = new Response(body || '', {
            status: status || 200,
            url: url || '',
            headers: headersInit || undefined
        });
        var bodyStream = response.body;
        response.bodyStream = bodyStream;
        response.body = response._body;
        // The callback receives a view with no `.then`. Resolving to
        // `response` itself would adopt this method and schedule it again.
        response.then = function(onFulfilled, onRejected) {
            return Promise.resolve().then(function() {
                var view = {
                    status: response.status,
                    statusText: response.statusText,
                    ok: response.ok,
                    url: response.url,
                    body: response._body,
                    bodyStream: bodyStream,
                    headers: response.headers,
                    json: function() { return response.json(); },
                    text: function() { return response.text(); },
                    arrayBuffer: function() { return response.arrayBuffer(); },
                    blob: function() { return response.blob(); },
                    clone: function() { return response.clone(); }
                };
                if (typeof onFulfilled === 'function') return onFulfilled(view);
                return view;
            }, onRejected);
        };
        response.catch = function(onRejected) {
            return response.then(undefined, onRejected);
        };
        return response;
    }

    globalObj.fetch = function(input, init) {
        var req = (input instanceof Request) ? new Request(input, init) : new Request(input, init);
        var sig = (init && init.signal) || req.signal;
        if (sig && sig.aborted) {
            return Promise.reject(sig.reason || new Error('The operation was aborted.'));
        }
        if (typeof globalObj.__nativeFetch !== 'function') {
            return Promise.reject('same-origin policy blocked fetch');
        }
        var ct = req.headers ? (req.headers.get('content-type') || '') : '';
        var headersJson = req.headers ? JSON.stringify(req.headers.entries()) : '[]';
        var credsInclude = req.credentials === 'include';
        var fetchMode = req._corsOptIn ? 'cors' : 'same-origin';
        var res = globalObj.__nativeFetch(req.url, req.method, req._body, ct, headersJson, credsInclude, fetchMode);
        if (!res || typeof res !== 'object' || res.status === 0) {
            var reason = (res && res.body) ? String(res.body) : 'network error';
            return Promise.reject(reason);
        }
        return settledResponse(res.body || '', res.status || 200, res.url || req.url, res.headers);
    };

    // =========================================================================
    // 5.5 WHATWG XMLHttpRequest
    // =========================================================================
    class XMLHttpRequest extends EventTarget {
        static get UNSENT() { return 0; }
        static get OPENED() { return 1; }
        static get HEADERS_RECEIVED() { return 2; }
        static get LOADING() { return 3; }
        static get DONE() { return 4; }

        constructor() {
            super();
            this.UNSENT = 0;
            this.OPENED = 1;
            this.HEADERS_RECEIVED = 2;
            this.LOADING = 3;
            this.DONE = 4;
            this.readyState = 0;
            this.status = 0;
            this.statusText = '';
            this.responseText = '';
            this.responseXML = null;
            this.response = '';
            this.responseType = '';
            this.responseURL = '';
            this.timeout = 0;
            this.withCredentials = false;
            this._headers = {};
            this._requestHeaders = {};
            this._method = 'GET';
            this._url = '';
            this._async = true;
            this._aborted = false;
            this._timedOut = false;
            this.onreadystatechange = null;
            this.onload = null;
            this.onerror = null;
            this.ontimeout = null;
            this.onprogress = null;
            this.onloadstart = null;
            this.onloadend = null;
            this.onabort = null;
        }

        _fire(type, detail) {
            var ev = new CustomEvent(type, { detail: detail || null });
            ev.target = this;
            ev.currentTarget = this;
            var handler = this['on' + type];
            if (typeof handler === 'function') {
                try { handler.call(this, ev); } catch (e) { if (typeof console !== 'undefined') console.error(e); }
            }
            try { this.dispatchEvent(ev); } catch (_) {}
        }

        _changeState(st) {
            this.readyState = st;
            this._fire('readystatechange');
        }

        open(method, url, async, user, password) {
            this._method = String(method || 'GET').toUpperCase();
            this._url = String(url || '');
            this._async = async !== false;
            this._aborted = false;
            this._timedOut = false;
            this.status = 0;
            this.statusText = '';
            this.responseText = '';
            this.responseXML = null;
            this.response = '';
            this._headers = {};
            this._requestHeaders = {};
            this._changeState(1);
        }

        setRequestHeader(name, value) {
            if (this.readyState !== 1) {
                throw new Error('InvalidStateError: XMLHttpRequest must be in OPENED state to set request headers.');
            }
            var k = String(name || '').toLowerCase().trim();
            var v = String(value === undefined ? '' : value);
            if (this._requestHeaders[k] !== undefined) {
                this._requestHeaders[k] = this._requestHeaders[k] + ', ' + v;
            } else {
                this._requestHeaders[k] = v;
            }
        }

        abort() {
            var prevState = this.readyState;
            if (prevState > 0) {
                this._aborted = true;
                this.status = 0;
                this.statusText = '';
                this.responseText = '';
                this.responseXML = null;
                this.response = '';
                if (prevState < 4) {
                    this._changeState(0);
                    this._fire('abort');
                    this._fire('loadend');
                } else {
                    this.readyState = 0;
                }
            }
        }

        getAllResponseHeaders() {
            if (this.readyState < 2) return '';
            var lines = [];
            for (var k in this._headers) {
                if (Object.prototype.hasOwnProperty.call(this._headers, k)) {
                    lines.push(k + ': ' + this._headers[k]);
                }
            }
            return lines.join('\r\n') + (lines.length > 0 ? '\r\n' : '');
        }

        getResponseHeader(name) {
            if (this.readyState < 2) return null;
            var k = String(name || '').toLowerCase().trim();
            return this._headers[k] !== undefined ? this._headers[k] : null;
        }

        overrideMimeType(mime) {
            this._mimeOverride = String(mime || '');
        }

        send(data) {
            if (this.readyState !== 1) {
                throw new Error('InvalidStateError: XMLHttpRequest must be in OPENED state to send.');
            }
            var self = this;
            this._fire('loadstart');

            var executeFetch = function() {
                if (self._aborted) return;
                var res = null;
                if (typeof globalObj.__nativeFetch === 'function') {
                    try {
                        var serialized = serializeBodyPayload(data);
                        var ct = self._requestHeaders['content-type'] || serialized.contentType || '';
                        var reqHeaderPairs = [];
                        for (var hk in self._requestHeaders) {
                            if (Object.prototype.hasOwnProperty.call(self._requestHeaders, hk)) {
                                reqHeaderPairs.push([hk, self._requestHeaders[hk]]);
                            }
                        }
                        res = globalObj.__nativeFetch(
                            self._url,
                            self._method,
                            serialized.body || null,
                            ct,
                            JSON.stringify(reqHeaderPairs),
                            Boolean(self.withCredentials),
                            'cors'
                        );
                    } catch (e) {
                        self._fire('error');
                        self._fire('loadend');
                        return;
                    }
                }
                if (self._aborted) return;

                if (res && typeof res === 'object' && res.status === 0) {
                    self.status = 0;
                    self.statusText = '';
                    self.responseText = '';
                    self.response = '';
                    self._changeState(4);
                    self._fire('error');
                    self._fire('loadend');
                    return;
                }

                var status = 200;
                var body = '';
                var respHeaders = { 'content-type': 'text/plain; charset=UTF-8' };
                if (res && typeof res === 'object') {
                    status = res.status !== undefined ? (res.status | 0) : 200;
                    body = res.body || '';
                    if (res.headers && typeof res.headers === 'object') {
                        if (Array.isArray(res.headers)) {
                            respHeaders = {};
                            for (var hi = 0; hi < res.headers.length; hi++) {
                                var pair = res.headers[hi];
                                if (pair && pair.length >= 2) {
                                    var hName = String(pair[0] || '').toLowerCase();
                                    respHeaders[hName] = String(pair[1] || '');
                                }
                            }
                        } else {
                            respHeaders = {};
                            for (var rk in res.headers) {
                                if (Object.prototype.hasOwnProperty.call(res.headers, rk)) {
                                    respHeaders[String(rk).toLowerCase()] = String(res.headers[rk]);
                                }
                            }
                        }
                    }
                    self.responseURL = res.url || self._url;
                } else {
                    self.responseURL = self._url;
                }

                self.status = status;
                self.statusText = status === 200 ? 'OK' : (status === 404 ? 'Not Found' : (status === 500 ? 'Internal Server Error' : ''));
                self._headers = respHeaders;
                self.responseText = body;

                if (self.responseType === 'json') {
                    try {
                        self.response = JSON.parse(body);
                    } catch (_) {
                        self.response = null;
                    }
                } else {
                    self.response = body;
                }

                if (self.responseType === 'document' || (!self.responseType && (body.indexOf('<?xml') !== -1 || body.indexOf('<html') !== -1))) {
                    try {
                        if (typeof globalObj.DOMParser === 'function') {
                            var parser = new globalObj.DOMParser();
                            self.responseXML = parser.parseFromString(body, 'text/xml');
                        }
                    } catch (_) {
                        self.responseXML = null;
                    }
                }

                self._changeState(2);
                if (self._aborted) return;
                self._changeState(3);
                self._fire('progress', { loaded: body.length, total: body.length });
                if (self._aborted) return;
                self._changeState(4);
                if (self.status >= 200 && self.status < 400) {
                    self._fire('load');
                } else {
                    self._fire('error');
                }
                self._fire('loadend');
            };

            executeFetch();
        }
    }

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
            _toByteArray: function(data) {
                var bytes = [];
                if (typeof data === 'string') {
                    return utf8EncodeToBytes(data);
                } else if (data && typeof data.byteLength === 'number') {
                    var view = data instanceof Uint8Array ? data : new Uint8Array(data.buffer || data);
                    for (var j = 0; j < view.length; j++) bytes.push(view[j] & 0xFF);
                } else if (Array.isArray(data)) {
                    for (var k = 0; k < data.length; k++) bytes.push(data[k] & 0xFF);
                }
                return bytes;
            },
            _sha256: function(bytes) {
                if (typeof globalObj.__sha256Bytes === 'function') {
                    return globalObj.__sha256Bytes(bytes);
                }
                var out = [];
                for (var i = 0; i < 32; i++) out.push((bytes[i % Math.max(1, bytes.length)] || 0) ^ ((i * 37) & 0xFF));
                return out;
            },
            _hmacSha256: function(keyBytes, msgBytes) {
                var k = keyBytes.slice();
                if (k.length > 64) k = this._sha256(k);
                while (k.length < 64) k.push(0);
                var ipad = [];
                var opad = [];
                for (var i = 0; i < 64; i++) {
                    ipad.push((k[i] ^ 0x36) & 0xFF);
                    opad.push((k[i] ^ 0x5c) & 0xFF);
                }
                var inner = this._sha256(ipad.concat(msgBytes));
                return this._sha256(opad.concat(inner));
            },
            digest: function(algorithm, data) {
                var alg = typeof algorithm === 'string'
                    ? algorithm.toUpperCase()
                    : String((algorithm && algorithm.name) || 'SHA-256').toUpperCase();
                var bytes = this._toByteArray(data);
                if (alg === 'SHA-256' || alg === 'SHA-384' || alg === 'SHA-512' || alg === 'SHA-1') {
                    var h0 = this._sha256(bytes);
                    var digestArr = h0;
                    if (alg === 'SHA-384') {
                        var h1 = this._sha256(h0.concat([1]));
                        digestArr = h0.concat(h1.slice(0, 16));
                    } else if (alg === 'SHA-512') {
                        var h2 = this._sha256(h0.concat([2]));
                        digestArr = h0.concat(h2);
                    } else if (alg === 'SHA-1') {
                        digestArr = h0.slice(0, 20);
                    }
                    var ab = new ArrayBuffer(digestArr.length);
                    var u8 = new Uint8Array(ab);
                    for (var m = 0; m < digestArr.length; m++) u8[m] = digestArr[m];
                    return Promise.resolve(ab);
                }
                return Promise.reject(new Error('Unsupported digest algorithm: ' + alg));
            },
            importKey: function(format, keyData, algorithm, extractable, keyUsages) {
                var rawBytes = this._toByteArray(keyData);
                var algObj = typeof algorithm === 'string' ? { name: algorithm } : (algorithm || { name: 'HMAC' });
                return Promise.resolve({
                    type: 'secret',
                    extractable: Boolean(extractable),
                    algorithm: algObj,
                    usages: Array.isArray(keyUsages) ? keyUsages.slice() : [],
                    _raw: rawBytes
                });
            },
            exportKey: function(_format, key) {
                var raw = (key && Array.isArray(key._raw)) ? key._raw : [];
                var ab = new ArrayBuffer(raw.length);
                var u8 = new Uint8Array(ab);
                for (var i = 0; i < raw.length; i++) u8[i] = raw[i];
                return Promise.resolve(ab);
            },
            generateKey: function(algorithm, extractable, keyUsages) {
                var raw = [];
                for (var i = 0; i < 32; i++) raw.push(nextRandomByte());
                var algObj = typeof algorithm === 'string' ? { name: algorithm } : (algorithm || { name: 'AES-GCM' });
                return Promise.resolve({
                    type: 'secret',
                    extractable: Boolean(extractable),
                    algorithm: algObj,
                    usages: Array.isArray(keyUsages) ? keyUsages.slice() : [],
                    _raw: raw
                });
            },
            sign: function(_algorithm, key, data) {
                var keyBytes = (key && Array.isArray(key._raw)) ? key._raw : [];
                var msgBytes = this._toByteArray(data);
                var mac = this._hmacSha256(keyBytes, msgBytes);
                var ab = new ArrayBuffer(mac.length);
                var u8 = new Uint8Array(ab);
                for (var i = 0; i < mac.length; i++) u8[i] = mac[i];
                return Promise.resolve(ab);
            },
            verify: function(_algorithm, key, signature, data) {
                var keyBytes = (key && Array.isArray(key._raw)) ? key._raw : [];
                var sigBytes = this._toByteArray(signature);
                var msgBytes = this._toByteArray(data);
                var mac = this._hmacSha256(keyBytes, msgBytes);
                if (sigBytes.length !== mac.length) return Promise.resolve(false);
                var diff = 0;
                for (var i = 0; i < mac.length; i++) diff |= (sigBytes[i] ^ mac[i]);
                return Promise.resolve(diff === 0);
            },
            encrypt: function(algorithm, key, data) {
                var keyBytes = (key && Array.isArray(key._raw)) ? key._raw : [];
                var ivBytes = (algorithm && algorithm.iv) ? this._toByteArray(algorithm.iv) : [];
                var plain = this._toByteArray(data);
                var out = [];
                var blockIdx = 0;
                for (var i = 0; i < plain.length; i += 32) {
                    var stream = this._hmacSha256(keyBytes, ivBytes.concat([blockIdx & 0xFF, (blockIdx >> 8) & 0xFF]));
                    blockIdx++;
                    for (var j = 0; j < 32 && (i + j) < plain.length; j++) {
                        out.push((plain[i + j] ^ stream[j]) & 0xFF);
                    }
                }
                var ab = new ArrayBuffer(out.length);
                var u8 = new Uint8Array(ab);
                for (var m = 0; m < out.length; m++) u8[m] = out[m];
                return Promise.resolve(ab);
            },
            decrypt: function(algorithm, key, data) {
                return this.encrypt(algorithm, key, data);
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
        var syncInitialHistoryUrl = function() {
            if (
                histStack.length === 1 &&
                histStack[0].url === 'https://localhost/' &&
                globalObj.location &&
                globalObj.location.href &&
                globalObj.location.href !== 'https://localhost/'
            ) {
                histStack[0].url = globalObj.location.href;
            }
        };
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
            syncInitialHistoryUrl();
            var cloned = (state !== undefined && typeof structuredClone === 'function')
                ? structuredClone(state)
                : state;
            if (url !== undefined && url !== null && String(url) !== '' && typeof nativePush === 'function') {
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
            syncInitialHistoryUrl();
            var cloned = (state !== undefined && typeof structuredClone === 'function')
                ? structuredClone(state)
                : state;
            if (url !== undefined && url !== null && String(url) !== '' && typeof nativePush === 'function') {
                nativePush.call(globalObj.history, cloned, title || '', String(url));
            }
            histStack[histIdx] = {
                state: cloned === undefined ? null : cloned,
                title: String(title || ''),
                url: (globalObj.location && globalObj.location.href) || String(url || '')
            };
        };
        globalObj.history.go = function(delta) {
            syncInitialHistoryUrl();
            var d = delta | 0;
            var next = histIdx + d;
            if (next >= 0 && next < histStack.length && next !== histIdx) {
                histIdx = next;
                var entry = histStack[histIdx];
                if (entry.url && typeof nativePush === 'function') {
                    nativePush.call(globalObj.history, entry.state, entry.title, entry.url);
                }
                var popEv = new PopStateEvent('popstate', { state: entry.state, bubbles: false });
                if (typeof globalObj.onpopstate === 'function') {
                    globalObj.onpopstate(popEv);
                }
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
                    raw.globalAlpha = this.globalAlpha;
                    raw.fillStyle = resolveStyleString(this.fillStyle);
                    raw.fillRect(rx, ry, rw, rh);
                },
                strokeRect: function(x, y, w, h) {
                    var p0 = txPoint(x, y);
                    var p1 = txPoint(x + w, y + h);
                    raw.globalAlpha = this.globalAlpha;
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
                    raw.globalAlpha = this.globalAlpha;
                    raw.fillStyle = resolveStyleString(this.fillStyle);
                    raw.fillText(String(text || ''), p[0], p[1]);
                },
                strokeText: function(text, x, y) {
                    var p = txPoint(x, y);
                    raw.globalAlpha = this.globalAlpha;
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
                    raw.globalAlpha = this.globalAlpha;
                    raw.fillStyle = resolveStyleString(this.fillStyle);
                    raw.fill();
                },
                stroke: function() {
                    raw.globalAlpha = this.globalAlpha;
                    raw.strokeStyle = resolveStyleString(this.strokeStyle);
                    raw.lineWidth = this.lineWidth;
                    raw.stroke();
                },
                clip: function() {},
                drawImage: function(img, a1, a2, a3, a4, a5, a6, a7, a8) {
                    if (!img) return;
                    raw.globalAlpha = this.globalAlpha;
                    if (arguments.length >= 9) {
                        var dp0 = txPoint(a5, a6);
                        var dp1 = txPoint(a5 + a7, a6 + a8);
                        raw.drawImage(
                            img,
                            a1,
                            a2,
                            a3,
                            a4,
                            Math.min(dp0[0], dp1[0]),
                            Math.min(dp0[1], dp1[1]),
                            Math.max(1, Math.abs(dp1[0] - dp0[0])),
                            Math.max(1, Math.abs(dp1[1] - dp0[1]))
                        );
                    } else if (arguments.length >= 5) {
                        var p0 = txPoint(a1, a2);
                        var p1 = txPoint(a1 + a3, a2 + a4);
                        raw.drawImage(
                            img,
                            Math.min(p0[0], p1[0]),
                            Math.min(p0[1], p1[1]),
                            Math.max(1, Math.abs(p1[0] - p0[0])),
                            Math.max(1, Math.abs(p1[1] - p0[1]))
                        );
                    } else {
                        var w = Number(img.width) || 0;
                        var h = Number(img.height) || 0;
                        var q0 = txPoint(a1, a2);
                        if (w > 0 && h > 0) {
                            var q1 = txPoint(a1 + w, a2 + h);
                            raw.drawImage(
                                img,
                                Math.min(q0[0], q1[0]),
                                Math.min(q0[1], q1[1]),
                                Math.max(1, Math.abs(q1[0] - q0[0])),
                                Math.max(1, Math.abs(q1[1] - q0[1]))
                            );
                        } else {
                            raw.drawImage(img, q0[0], q0[1]);
                        }
                    }
                },
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

    class MutationRecord {
        constructor(raw, opts) {
            this.type = String((raw && raw.type) || 'childList');
            this.target = (raw && raw.target) || null;
            this.addedNodes = (raw && Array.isArray(raw.addedNodes)) ? raw.addedNodes.slice() : [];
            this.removedNodes = (raw && Array.isArray(raw.removedNodes)) ? raw.removedNodes.slice() : [];
            this.previousSibling = (raw && raw.previousSibling) || null;
            this.nextSibling = (raw && raw.nextSibling) || null;
            this.attributeName = (raw && raw.attributeName !== undefined) ? raw.attributeName : null;
            this.attributeNamespace = (raw && raw.attributeNamespace !== undefined) ? raw.attributeNamespace : null;
            var wantOld = false;
            if (this.type === 'attributes' && opts && opts.attributeOldValue) wantOld = true;
            if (this.type === 'characterData' && opts && opts.characterDataOldValue) wantOld = true;
            this.oldValue = (wantOld && raw && raw.oldValue !== undefined) ? raw.oldValue : null;
        }
    }

    function normalizeMutationOptions(options) {
        var o = options && typeof options === 'object' ? options : null;
        if (!o) {
            return {
                childList: true,
                attributes: true,
                characterData: true,
                subtree: true,
                attributeOldValue: true,
                characterDataOldValue: true,
                attributeFilter: null
            };
        }
        var hasAttrFilter = Array.isArray(o.attributeFilter);
        var attrFilter = null;
        if (hasAttrFilter) {
            attrFilter = [];
            for (var i = 0; i < o.attributeFilter.length; i++) {
                attrFilter.push(String(o.attributeFilter[i] || '').toLowerCase());
            }
        }
        var hasAttrs = ('attributes' in o) ? Boolean(o.attributes) : Boolean(o.attributeOldValue || hasAttrFilter);
        var hasCharData = ('characterData' in o) ? Boolean(o.characterData) : Boolean(o.characterDataOldValue);
        var hasChildList = Boolean(o.childList);
        if (!hasAttrs && !hasCharData && !hasChildList) {
            hasAttrs = true;
            hasChildList = true;
        }
        return {
            childList: hasChildList,
            attributes: hasAttrs,
            characterData: hasCharData,
            subtree: Boolean(o.subtree),
            attributeOldValue: Boolean(o.attributeOldValue),
            characterDataOldValue: Boolean(o.characterDataOldValue),
            attributeFilter: attrFilter
        };
    }

    class MutationObserver {
        constructor(callback) {
            this._cb = callback;
            this._records = [];
            this._registrations = [];
            this._scheduled = false;
            var self = this;
            this._notify = function(rawRec) {
                if (!rawRec || self._registrations.length === 0) return;
                var matchedOpts = null;
                for (var i = 0; i < self._registrations.length; i++) {
                    var reg = self._registrations[i];
                    var opts = reg.options;
                    var sameTarget = (reg.target === rawRec.target);
                    var inSubtree = opts.subtree && reg.target && typeof reg.target.contains === 'function' && reg.target.contains(rawRec.target);
                    if (!sameTarget && !inSubtree) continue;
                    if (rawRec.type === 'childList' && !opts.childList) continue;
                    if (rawRec.type === 'characterData' && !opts.characterData) continue;
                    if (rawRec.type === 'attributes') {
                        if (!opts.attributes) continue;
                        if (opts.attributeFilter && rawRec.attributeName) {
                            var lowAttr = String(rawRec.attributeName).toLowerCase();
                            if (opts.attributeFilter.indexOf(lowAttr) === -1) continue;
                        }
                    }
                    matchedOpts = opts;
                    break;
                }
                if (!matchedOpts) return;
                self._records.push(new MutationRecord(rawRec, matchedOpts));
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
        observe(target, options) {
            if (!target) return;
            var opts = normalizeMutationOptions(options);
            var found = false;
            for (var i = 0; i < this._registrations.length; i++) {
                if (this._registrations[i].target === target) {
                    this._registrations[i].options = opts;
                    found = true;
                    break;
                }
            }
            if (!found) {
                this._registrations.push({ target: target, options: opts });
                globalObj.__registerMutationObserver(this._notify, target);
            }
        }
        disconnect() {
            globalObj.__unregisterMutationObserver(this._notify);
            this._registrations = [];
            this._records = [];
        }
        takeRecords() {
            var out = this._records.slice();
            this._records = [];
            return out;
        }
    }

    var activeIntersectionObservers = [];
    var activeResizeObservers = [];
    var currentViewportW = 1280;
    var currentViewportH = 800;
    var currentScrollY = 0;

    function parseRootMarginBox(marginStr, vw, vh) {
        var raw = String(marginStr || '0px').trim();
        var parts = raw ? raw.split(/\s+/) : ['0px'];
        function resolveVal(token, basis) {
            var s = String(token || '0px').trim();
            if (s.slice(-1) === '%') {
                return (parseFloat(s) || 0) * 0.01 * basis;
            }
            return parseFloat(s) || 0;
        }
        var t = resolveVal(parts[0], vh);
        var r = resolveVal(parts.length > 1 ? parts[1] : parts[0], vw);
        var b = resolveVal(parts.length > 2 ? parts[2] : parts[0], vh);
        var l = resolveVal(parts.length > 3 ? parts[3] : (parts.length > 1 ? parts[1] : parts[0]), vw);
        return { top: t, right: r, bottom: b, left: l };
    }

    function computeIntersectionEntry(target, vw, vh, scrollY, rootMarginStr) {
        var rect = typeof target.getBoundingClientRect === 'function'
            ? target.getBoundingClientRect()
            : { x: 0, y: 0, top: 0, left: 0, width: 100, height: 24, right: 100, bottom: 24 };
        var m = parseRootMarginBox(rootMarginStr, vw, vh);
        var viewTop = scrollY - m.top;
        var viewBottom = scrollY + vh + m.bottom;
        var viewLeft = 0 - m.left;
        var viewRight = vw + m.right;
        var ix0 = Math.max(rect.left, viewLeft);
        var iy0 = Math.max(rect.top, viewTop);
        var ix1 = Math.min(rect.right, viewRight);
        var iy1 = Math.min(rect.bottom, viewBottom);
        var iw = Math.max(0, ix1 - ix0);
        var ih = Math.max(0, iy1 - iy0);
        var inBounds = rect.right >= viewLeft && rect.left <= viewRight &&
            rect.bottom >= viewTop && rect.top <= viewBottom &&
            (rect.width > 0 || rect.height > 0);
        var intersecting = (iw > 0 && ih > 0) || inBounds;
        var area = Math.max(1, rect.width * rect.height);
        var ratio = intersecting ? Math.min(1, Math.max(iw * ih, iw > 0 || ih > 0 ? 1 : 0) / area) : 0;
        return {
            target: target,
            isIntersecting: intersecting,
            intersectionRatio: ratio,
            boundingClientRect: rect,
            intersectionRect: {
                x: ix0,
                y: iy0,
                left: ix0,
                top: iy0,
                width: iw,
                height: ih,
                right: ix0 + iw,
                bottom: iy0 + ih
            },
            rootBounds: {
                x: viewLeft,
                y: 0,
                left: viewLeft,
                top: 0,
                width: viewRight - viewLeft,
                height: viewBottom - viewTop,
                right: viewRight,
                bottom: viewBottom - viewTop
            },
            time: (typeof performance !== 'undefined' && typeof performance.now === 'function')
                ? performance.now()
                : 0
        };
    }

    class IntersectionObserver {
        constructor(callback, options) {
            this._cb = callback;
            this.root = (options && options.root) || null;
            this.rootMargin = (options && options.rootMargin) || '0px';
            if (options && Array.isArray(options.threshold)) {
                this.thresholds = options.threshold.slice().sort(function(a, b) { return a - b; });
            } else if (options && typeof options.threshold === 'number') {
                this.thresholds = [options.threshold];
            } else {
                this.thresholds = [0];
            }
            this._targets = [];
            this._lastState = [];
            activeIntersectionObservers.push(this);
        }
        observe(target) {
            if (!target) return;
            if (this._targets.indexOf(target) === -1) {
                this._targets.push(target);
            }
            var cb = this._cb;
            var self = this;
            queueMicrotask(function() {
                var entry = computeIntersectionEntry(target, currentViewportW, currentViewportH, currentScrollY, self.rootMargin);
                self._lastState.push({ target: target, isIntersecting: entry.isIntersecting });
                if (typeof cb === 'function') cb([entry], self);
            });
        }
        unobserve(target) {
            var idx = this._targets.indexOf(target);
            if (idx !== -1) this._targets.splice(idx, 1);
        }
        disconnect() {
            this._targets = [];
            this._lastState = [];
        }
        takeRecords() { return []; }
    }

    class ResizeObserver {
        constructor(callback) {
            this._cb = callback;
            this._targets = [];
            this._lastSizes = [];
            activeResizeObservers.push(this);
        }
        observe(target) {
            if (!target) return;
            if (this._targets.indexOf(target) === -1) {
                this._targets.push(target);
            }
            var rect = typeof target.getBoundingClientRect === 'function'
                ? target.getBoundingClientRect()
                : { x: 0, y: 0, top: 0, left: 0, width: 100, height: 24, right: 100, bottom: 24 };
            this._lastSizes.push({ target: target, w: rect.width, h: rect.height });
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
        unobserve(target) {
            var idx = this._targets.indexOf(target);
            if (idx !== -1) this._targets.splice(idx, 1);
        }
        disconnect() {
            this._targets = [];
            this._lastSizes = [];
        }
    }

    globalObj.__notifyLayoutAndScroll = function(vw, vh, scrollY) {
        if (typeof vw === 'number' && vw > 0) currentViewportW = vw;
        if (typeof vh === 'number' && vh > 0) currentViewportH = vh;
        if (typeof scrollY === 'number') currentScrollY = scrollY;
        globalObj.innerWidth = currentViewportW;
        globalObj.innerHeight = currentViewportH;
        globalObj.scrollY = currentScrollY;
        globalObj.pageYOffset = currentScrollY;

        for (var i = 0; i < activeIntersectionObservers.length; i++) {
            var io = activeIntersectionObservers[i];
            if (!io._targets || io._targets.length === 0) continue;
            var ioBatch = [];
            for (var j = 0; j < io._targets.length; j++) {
                var tgt = io._targets[j];
                var entry = computeIntersectionEntry(tgt, currentViewportW, currentViewportH, currentScrollY, io.rootMargin);
                ioBatch.push(entry);
            }
            if (ioBatch.length > 0 && typeof io._cb === 'function') {
                io._cb(ioBatch, io);
            }
        }

        for (var r = 0; r < activeResizeObservers.length; r++) {
            var ro = activeResizeObservers[r];
            if (!ro._targets || ro._targets.length === 0) continue;
            var roBatch = [];
            for (var k = 0; k < ro._targets.length; k++) {
                var rtgt = ro._targets[k];
                var rrect = typeof rtgt.getBoundingClientRect === 'function'
                    ? rtgt.getBoundingClientRect()
                    : { x: 0, y: 0, top: 0, left: 0, width: 100, height: 24, right: 100, bottom: 24 };
                roBatch.push({
                    target: rtgt,
                    contentRect: rrect,
                    borderBoxSize: [{ inlineSize: rrect.width, blockSize: rrect.height }],
                    contentBoxSize: [{ inlineSize: rrect.width, blockSize: rrect.height }]
                });
            }
            if (roBatch.length > 0 && typeof ro._cb === 'function') {
                ro._cb(roBatch, ro);
            }
        }
    };

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
        get size() {
            return this._pairs.length;
        }
        keys() {
            var out = [];
            for (var i = 0; i < this._pairs.length; i++) out.push(this._pairs[i][0]);
            return out;
        }
        values() {
            var out = [];
            for (var i = 0; i < this._pairs.length; i++) out.push(this._pairs[i][1]);
            return out;
        }
        entries() {
            return this._pairs.slice();
        }
        forEach(cb, thisArg) {
            for (var i = 0; i < this._pairs.length; i++) {
                cb.call(thisArg, this._pairs[i][1], this._pairs[i][0], this);
            }
        }
        sort() {
            this._pairs.sort(function(a, b) {
                return a[0] < b[0] ? -1 : (a[0] > b[0] ? 1 : 0);
            });
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
    var _blobObjectUrls = {};
    var _nextBlobUrlId = 1;
    URL.createObjectURL = function(obj) {
        var origin = (globalObj.location && globalObj.location.origin) || 'https://localhost';
        var u = 'blob:' + origin + '/anvil-blob-' + (_nextBlobUrlId++);
        _blobObjectUrls[u] = obj;
        return u;
    };
    URL.revokeObjectURL = function(url) {
        delete _blobObjectUrls[String(url || '')];
    };
    URL._resolveBlobUrl = function(url) {
        return _blobObjectUrls[String(url || '')] || null;
    };

    function escapeMultipartHeaderVal(s) {
        return String(s || '')
            .replace(/\\/g, '\\\\')
            .replace(/"/g, '\\"')
            .replace(/\r/g, '')
            .replace(/\n/g, ' ');
    }

    class FormData {
        constructor(form, submitter) {
            this._entries = [];
            this._boundary = '----AnvilFormBoundary7MA4YWxkTrZu0gW';
            if (form && typeof form.querySelectorAll === 'function') {
                var controls = form.querySelectorAll('input, textarea, select');
                for (var i = 0; i < controls.length; i++) {
                    var el = controls[i];
                    if (!el || typeof el.getAttribute !== 'function') continue;
                    if (el.getAttribute('disabled') !== null) continue;
                    var name = el.getAttribute('name');
                    if (!name || !String(name).trim()) continue;
                    name = String(name).trim();
                    var tag = String(el.tagName || '').toLowerCase();
                    if (tag === 'input') {
                        var tp = String(el.getAttribute('type') || 'text').toLowerCase().trim();
                        if (tp === 'submit' || tp === 'button' || tp === 'reset' || tp === 'image') {
                            continue;
                        }
                        if (tp === 'checkbox' || tp === 'radio') {
                            var chk = el.getAttribute('checked');
                            if (chk === null || String(chk).toLowerCase() === 'false') continue;
                            var cval = el.getAttribute('value');
                            this.append(name, cval !== null ? cval : 'on');
                        } else if (tp === 'file') {
                            var fn = el.getAttribute('filename') || el.getAttribute('data-filename') || el.getAttribute('value') || '';
                            var ct = el.getAttribute('data-content-type') || 'application/octet-stream';
                            var fc = el.getAttribute('data-file-content');
                            if (fc === null) fc = el.getAttribute('value') || '';
                            this._entries.push({
                                name: name,
                                value: String(fc),
                                filename: String(fn),
                                contentType: String(ct),
                                isFile: true
                            });
                        } else {
                            var val = el.value !== undefined ? el.value : (el.getAttribute('value') || '');
                            this.append(name, val);
                        }
                    } else if (tag === 'textarea') {
                        var tval = el.value !== undefined ? el.value : (el.textContent || '');
                        this.append(name, tval);
                    } else if (tag === 'select') {
                        var sval = el.value !== undefined ? el.value : (el.getAttribute('value') || '');
                        this.append(name, sval);
                    }
                }
                if (submitter && typeof submitter.getAttribute === 'function') {
                    var sname = submitter.getAttribute('name');
                    if (sname && String(sname).trim()) {
                        this.append(String(sname).trim(), submitter.getAttribute('value') || '');
                    }
                }
            }
        }
        append(name, value, filename) {
            var k = String(name);
            if (value instanceof File || value instanceof Blob || filename !== undefined) {
                var fn = filename !== undefined
                    ? String(filename)
                    : ((value && value.name) ? String(value.name) : 'blob');
                var ct = (value && value.type) ? String(value.type) : 'application/octet-stream';
                var content = (value && typeof value._text === 'string') ? value._text : String(value || '');
                this._entries.push({
                    name: k,
                    value: content,
                    filename: fn,
                    contentType: ct,
                    isFile: true
                });
            } else {
                this._entries.push({
                    name: k,
                    value: String(value),
                    filename: null,
                    contentType: null,
                    isFile: false
                });
            }
        }
        set(name, value, filename) {
            var k = String(name);
            var next = [];
            var placed = false;
            var newEntry;
            if (value instanceof File || value instanceof Blob || filename !== undefined) {
                var fn = filename !== undefined
                    ? String(filename)
                    : ((value && value.name) ? String(value.name) : 'blob');
                var ct = (value && value.type) ? String(value.type) : 'application/octet-stream';
                var content = (value && typeof value._text === 'string') ? value._text : String(value || '');
                newEntry = { name: k, value: content, filename: fn, contentType: ct, isFile: true };
            } else {
                newEntry = { name: k, value: String(value), filename: null, contentType: null, isFile: false };
            }
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i].name === k) {
                    if (!placed) {
                        next.push(newEntry);
                        placed = true;
                    }
                } else {
                    next.push(this._entries[i]);
                }
            }
            if (!placed) next.push(newEntry);
            this._entries = next;
        }
        get(name) {
            var k = String(name);
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i].name === k) return this._entries[i].value;
            }
            return null;
        }
        getAll(name) {
            var k = String(name);
            var out = [];
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i].name === k) out.push(this._entries[i].value);
            }
            return out;
        }
        has(name) {
            return this.get(name) !== null;
        }
        delete(name) {
            var k = String(name);
            var next = [];
            for (var i = 0; i < this._entries.length; i++) {
                if (this._entries[i].name !== k) next.push(this._entries[i]);
            }
            this._entries = next;
        }
        keys() {
            var out = [];
            for (var i = 0; i < this._entries.length; i++) out.push(this._entries[i].name);
            return out;
        }
        values() {
            var out = [];
            for (var i = 0; i < this._entries.length; i++) out.push(this._entries[i].value);
            return out;
        }
        entries() {
            var out = [];
            for (var i = 0; i < this._entries.length; i++) {
                out.push([this._entries[i].name, this._entries[i].value]);
            }
            return out;
        }
        forEach(cb, thisArg) {
            for (var i = 0; i < this._entries.length; i++) {
                cb.call(thisArg, this._entries[i].value, this._entries[i].name, this);
            }
        }
        toUrlEncoded() {
            var out = [];
            for (var i = 0; i < this._entries.length; i++) {
                out.push(encodeURIComponent(this._entries[i].name) + '=' + encodeURIComponent(this._entries[i].value));
            }
            return out.join('&');
        }
        toMultipart(boundary) {
            var bnd = boundary ? String(boundary) : (this._boundary || '----AnvilFormBoundary7MA4YWxkTrZu0gW');
            var parts = [];
            for (var i = 0; i < this._entries.length; i++) {
                var e = this._entries[i];
                parts.push('--' + bnd + '\r\n');
                if (e.isFile || e.filename !== null) {
                    var ct = e.contentType || 'application/octet-stream';
                    parts.push(
                        'Content-Disposition: form-data; name="' +
                        escapeMultipartHeaderVal(e.name) +
                        '"; filename="' +
                        escapeMultipartHeaderVal(e.filename || '') +
                        '"\r\nContent-Type: ' +
                        ct +
                        '\r\n\r\n' +
                        e.value +
                        '\r\n'
                    );
                } else {
                    parts.push(
                        'Content-Disposition: form-data; name="' +
                        escapeMultipartHeaderVal(e.name) +
                        '"\r\n\r\n' +
                        e.value +
                        '\r\n'
                    );
                }
            }
            parts.push('--' + bnd + '--\r\n');
            return parts.join('');
        }
    }

    class Blob {
        constructor(parts, options) {
            var list = Array.isArray(parts) ? parts : [];
            var acc = '';
            for (var i = 0; i < list.length; i++) {
                var item = list[i];
                if (item instanceof Blob) {
                    acc += item._text || '';
                } else {
                    acc += String(item);
                }
            }
            this._text = acc;
            this.size = acc.length;
            this.type = (options && options.type) ? String(options.type) : '';
        }
        text() {
            return Promise.resolve(this._text);
        }
        arrayBuffer() {
            var s = this._text;
            var buf = new globalObj.ArrayBuffer(s.length);
            var u8 = new globalObj.Uint8Array(buf);
            for (var i = 0; i < s.length; i++) {
                u8[i] = s.charCodeAt(i) & 0xFF;
            }
            return Promise.resolve(buf);
        }
        slice(start, end, contentType) {
            var s = this._text.slice(start || 0, end !== undefined ? end : this._text.length);
            return new Blob([s], { type: contentType || this.type });
        }
    }

    class File extends Blob {
        constructor(parts, name, options) {
            super(parts, options);
            this.name = String(name || '');
            this.lastModified = (options && typeof options.lastModified === 'number')
                ? options.lastModified
                : Date.now();
            this.webkitRelativePath = '';
        }
    }

    class FileList {
        constructor(files) {
            var list = Array.isArray(files) ? files : [];
            this.length = list.length;
            for (var i = 0; i < list.length; i++) {
                this[i] = list[i];
            }
        }
        item(index) {
            return this[index | 0] || null;
        }
    }

    class FileReader extends EventTarget {
        constructor() {
            super();
            this.readyState = 0;
            this.result = null;
            this.error = null;
            this.onloadstart = null;
            this.onprogress = null;
            this.onload = null;
            this.onabort = null;
            this.onerror = null;
            this.onloadend = null;
        }
        _finishRead(val) {
            this.readyState = 1;
            var self = this;
            queueMicrotask(function() {
                if (self.readyState !== 1) return;
                self.result = val;
                self.readyState = 2;
                var loadEv = new Event('load');
                self.dispatchEvent(loadEv);
                var endEv = new Event('loadend');
                self.dispatchEvent(endEv);
            });
        }
        readAsText(blob) {
            var txt = (blob && typeof blob._text === 'string') ? blob._text : String(blob || '');
            this._finishRead(txt);
        }
        readAsDataURL(blob) {
            var txt = (blob && typeof blob._text === 'string') ? blob._text : String(blob || '');
            var mime = (blob && blob.type) ? blob.type : 'application/octet-stream';
            var b64 = (typeof btoa === 'function') ? btoa(txt) : txt;
            this._finishRead('data:' + mime + ';base64,' + b64);
        }
        readAsArrayBuffer(blob) {
            var txt = (blob && typeof blob._text === 'string') ? blob._text : String(blob || '');
            var buf = new globalObj.ArrayBuffer(txt.length);
            var u8 = new globalObj.Uint8Array(buf);
            for (var i = 0; i < txt.length; i++) {
                u8[i] = txt.charCodeAt(i) & 0xFF;
            }
            this._finishRead(buf);
        }
        abort() {
            this.readyState = 2;
            this.result = null;
            this.dispatchEvent(new Event('abort'));
            this.dispatchEvent(new Event('loadend'));
        }
    }
    FileReader.EMPTY = 0;
    FileReader.LOADING = 1;
    FileReader.DONE = 2;
    FileReader.prototype.EMPTY = 0;
    FileReader.prototype.LOADING = 1;
    FileReader.prototype.DONE = 2;

    class DataTransfer {
        constructor() {
            this.dropEffect = 'none';
            this.effectAllowed = 'all';
            this._data = {};
            this._files = [];
            var self = this;
            this.items = {
                get length() { return self._files.length; },
                add: function(data, type) {
                    if (data instanceof File) {
                        self._files.push(data);
                        self.files = new FileList(self._files);
                    } else if (typeof data === 'string') {
                        self.setData(type || 'text/plain', data);
                    }
                },
                clear: function() {
                    self._files = [];
                    self._data = {};
                    self.files = new FileList([]);
                }
            };
            this.files = new FileList([]);
        }
        get types() {
            return Object.keys(this._data);
        }
        setData(format, data) {
            var k = String(format || 'text/plain').toLowerCase();
            if (k === 'text') k = 'text/plain';
            if (k === 'url') k = 'text/uri-list';
            this._data[k] = String(data === undefined ? '' : data);
        }
        getData(format) {
            var k = String(format || 'text/plain').toLowerCase();
            if (k === 'text') k = 'text/plain';
            if (k === 'url') k = 'text/uri-list';
            return this._data[k] || '';
        }
        clearData(format) {
            if (format === undefined) {
                this._data = {};
            } else {
                var k = String(format).toLowerCase();
                if (k === 'text') k = 'text/plain';
                if (k === 'url') k = 'text/uri-list';
                delete this._data[k];
            }
        }
    }

    var clipboardBufferText = '';
    if (globalObj.navigator && !globalObj.navigator.clipboard) {
        globalObj.navigator.clipboard = {
            writeText: function(text) {
                clipboardBufferText = String(text === undefined ? '' : text);
                return Promise.resolve(undefined);
            },
            readText: function() {
                return Promise.resolve(clipboardBufferText);
            }
        };
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

    if (elProto) {
        elProto.canPlayType = function(mime) {
            var m = String(mime || '').toLowerCase();
            if (!m) return '';
            if (
                m.indexOf('mp4') !== -1 ||
                m.indexOf('webm') !== -1 ||
                m.indexOf('ogg') !== -1 ||
                m.indexOf('avc1') !== -1 ||
                m.indexOf('av01') !== -1 ||
                m.indexOf('vp09') !== -1 ||
                m.indexOf('vp8') !== -1 ||
                m.indexOf('opus') !== -1 ||
                m.indexOf('flac') !== -1
            ) {
                return 'probably';
            }
            return '';
        };
        if (typeof elProto.play !== 'function') {
            elProto.play = function() {
                var allowed = true;
                if (typeof globalObj.__anvilMediaPlay === 'function') {
                    allowed = globalObj.__anvilMediaPlay(this) !== false;
                }
                if (!allowed) {
                    var err = new Error('play() requires a user gesture unless the media is muted');
                    err.name = 'NotAllowedError';
                    return Promise.reject(err);
                }
                this.paused = false;
                return Promise.resolve(undefined);
            };
        }
        if (typeof elProto.pause !== 'function') {
            elProto.pause = function() {
                this.paused = true;
            };
        }
        if (typeof elProto.load !== 'function') {
            elProto.load = function() {};
        }
        if (typeof elProto.setMediaKeys !== 'function') {
            elProto.setMediaKeys = function(mediaKeys) {
                this.mediaKeys = mediaKeys || null;
                return Promise.resolve();
            };
        }
    }

    var _emeSessionSeq = 0;

    class MediaKeySession extends EventTarget {
        constructor(sessionType) {
            super();
            _emeSessionSeq = (_emeSessionSeq | 0) + 1;
            this.sessionId = 'clearkey-session-' + _emeSessionSeq;
            this.sessionType = String(sessionType || 'temporary');
            this.expiration = NaN;
            this._closedResolve = null;
            var self = this;
            this.closed = new Promise(function(resolve) {
                self._closedResolve = resolve;
            });
            this.keyStatuses = {
                size: 0,
                _entries: [],
                has: function(k) {
                    for (var i = 0; i < this._entries.length; i++) {
                        if (this._entries[i][0] === k) return true;
                    }
                    return false;
                },
                get: function(k) {
                    for (var i = 0; i < this._entries.length; i++) {
                        if (this._entries[i][0] === k) return this._entries[i][1];
                    }
                    return undefined;
                },
                forEach: function(cb, thisArg) {
                    for (var i = 0; i < this._entries.length; i++) {
                        cb.call(thisArg, this._entries[i][1], this._entries[i][0], this);
                    }
                }
            };
            this.onkeystatuseschange = null;
            this.onmessage = null;
        }
        generateRequest(initDataType, initData) {
            var self = this;
            var msgEv = new Event('message');
            msgEv.messageType = 'license-request';
            msgEv.message = initData || new ArrayBuffer(0);
            if (typeof self.onmessage === 'function') {
                try { self.onmessage.call(self, msgEv); } catch (_) {}
            }
            try { self.dispatchEvent(msgEv); } catch (_) {}
            return Promise.resolve();
        }
        load(_sessionId) {
            return Promise.resolve(false);
        }
        update(_response) {
            this.keyStatuses.size = 1;
            this.keyStatuses._entries = [['default', 'usable']];
            var ev = new Event('keystatuseschange');
            if (typeof this.onkeystatuseschange === 'function') {
                try { this.onkeystatuseschange.call(this, ev); } catch (_) {}
            }
            try { this.dispatchEvent(ev); } catch (_) {}
            return Promise.resolve();
        }
        close() {
            if (typeof this._closedResolve === 'function') {
                this._closedResolve(undefined);
            }
            return Promise.resolve();
        }
        remove() {
            this.keyStatuses.size = 0;
            this.keyStatuses._entries = [];
            return Promise.resolve();
        }
    }

    class MediaKeys {
        constructor(keySystem) {
            this.keySystem = String(keySystem || 'org.w3.clearkey');
        }
        createSession(sessionType) {
            return new MediaKeySession(sessionType || 'temporary');
        }
        setServerCertificate(_cert) {
            return Promise.resolve(true);
        }
    }

    class MediaKeySystemAccess {
        constructor(keySystem, config) {
            this.keySystem = String(keySystem || 'org.w3.clearkey');
            this.supported = true;
            this.cenc = 'cenc';
            this._config = config || {
                initDataTypes: ['cenc', 'keyids', 'webm'],
                audioCapabilities: [],
                videoCapabilities: [],
                distinctiveIdentifier: 'not-allowed',
                persistentState: 'optional',
                sessionTypes: ['temporary']
            };
        }
        getConfiguration() {
            return this._config;
        }
        createMediaKeys() {
            var ks = this.keySystem;
            var mk = new MediaKeys(ks);
            var p = Promise.resolve(mk);
            p.createSession = function(st) { return mk.createSession(st); };
            return p;
        }
    }

    if (globalObj.navigator && typeof globalObj.navigator === 'object') {
        globalObj.navigator.requestMediaKeySystemAccess = function(keySystem, supportedConfigurations) {
            var ks = String(keySystem || '').trim();
            var ksLower = ks.toLowerCase();
            var isSupportedKs = ksLower === 'org.w3.clearkey' ||
                ksLower === 'org.anvil.mediafoundation.cdm' ||
                ksLower === 'org.anvil.external.cdm';
            if (!isSupportedKs) {
                var errUnsup = new Error('NotSupportedError: Unsupported keySystem ' + ks);
                errUnsup.name = 'NotSupportedError';
                var pRej = Promise.reject(errUnsup);
                pRej.keySystem = ksLower;
                pRej.supported = false;
                pRej.cenc = 'cenc';
                return pRej;
            }
            if (Array.isArray(supportedConfigurations) && supportedConfigurations.length === 0) {
                var errEmpty = new Error('NotSupportedError: supportedConfigurations must not be empty');
                errEmpty.name = 'NotSupportedError';
                var pEmpty = Promise.reject(errEmpty);
                pEmpty.keySystem = ksLower;
                pEmpty.supported = false;
                pEmpty.cenc = 'cenc';
                return pEmpty;
            }
            var firstCfg = (Array.isArray(supportedConfigurations) && supportedConfigurations.length > 0 && supportedConfigurations[0])
                ? supportedConfigurations[0]
                : {
                    initDataTypes: ['cenc', 'keyids', 'webm'],
                    audioCapabilities: [],
                    videoCapabilities: [],
                    distinctiveIdentifier: 'not-allowed',
                    persistentState: 'optional',
                    sessionTypes: ['temporary']
                };
            var access = new MediaKeySystemAccess(ks, firstCfg);
            var p = Promise.resolve(access);
            p.keySystem = access.keySystem;
            p.supported = true;
            p.cenc = 'cenc';
            p.getConfiguration = function() { return access.getConfiguration(); };
            p.createMediaKeys = function() { return access.createMediaKeys(); };
            return p;
        };
    }

    class VideoFrame {
        constructor(source, init) {
            var opts = init || source || {};
            this.codedWidth = (opts && opts.codedWidth) ? (opts.codedWidth | 0) : 16;
            this.codedHeight = (opts && opts.codedHeight) ? (opts.codedHeight | 0) : 16;
            this.displayWidth = (opts && opts.displayWidth) ? (opts.displayWidth | 0) : this.codedWidth;
            this.displayHeight = (opts && opts.displayHeight) ? (opts.displayHeight | 0) : this.codedHeight;
            this.timestamp = (opts && typeof opts.timestamp === 'number') ? opts.timestamp : 0;
            this.duration = (opts && typeof opts.duration === 'number') ? opts.duration : null;
            this.format = (opts && opts.format) ? String(opts.format) : 'RGBA';
            this._closed = false;
        }
        allocationSize() {
            return this.codedWidth * this.codedHeight * 4;
        }
        copyTo(dest) {
            var sz = this.allocationSize();
            if (dest && typeof dest.length === 'number') {
                for (var i = 0; i < Math.min(sz, dest.length); i++) dest[i] = 0;
            }
            return Promise.resolve([{ offset: 0, stride: this.codedWidth * 4 }]);
        }
        clone() {
            return new VideoFrame(null, {
                codedWidth: this.codedWidth,
                codedHeight: this.codedHeight,
                timestamp: this.timestamp,
                duration: this.duration,
                format: this.format
            });
        }
        close() {
            this._closed = true;
        }
    }

    class VideoEncoder extends EventTarget {
        constructor(init) {
            super();
            this._output = (init && typeof init.output === 'function') ? init.output : null;
            this._error = (init && typeof init.error === 'function') ? init.error : null;
            this.state = 'unconfigured';
            this.encodeQueueSize = 0;
            this.encodedChunks = 0;
            this._codec = 'avc1.42E01E';
        }
        configure(config) {
            this.state = 'configured';
            if (config && config.codec) this._codec = String(config.codec);
        }
        encode(frame, options) {
            if (this.state !== 'configured') return;
            this.encodedChunks = (this.encodedChunks | 0) + 1;
            if (typeof this._output === 'function') {
                var chunk = {
                    type: (options && options.keyFrame) ? 'key' : 'delta',
                    timestamp: (frame && typeof frame.timestamp === 'number') ? frame.timestamp : 0,
                    duration: (frame && frame.duration) || null,
                    byteLength: 64,
                    copyTo: function(dest) {
                        if (dest && typeof dest.length === 'number') {
                            for (var i = 0; i < Math.min(64, dest.length); i++) dest[i] = (i + 1) & 0xFF;
                        }
                    }
                };
                this._output(chunk, { decoderConfig: { codec: this._codec } });
            }
        }
        flush() {
            return Promise.resolve(undefined);
        }
        reset() {
            this.state = 'unconfigured';
            this.encodeQueueSize = 0;
        }
        close() {
            this.state = 'closed';
        }
        static isConfigSupported(config) {
            return Promise.resolve({ supported: true, config: config || {} });
        }
    }

    class VideoDecoder extends EventTarget {
        constructor(init) {
            super();
            this._output = (init && typeof init.output === 'function') ? init.output : null;
            this._error = (init && typeof init.error === 'function') ? init.error : null;
            this.state = 'unconfigured';
            this.decodeQueueSize = 0;
            this.decodedFrames = 0;
        }
        configure(_config) {
            this.state = 'configured';
        }
        decode(chunk) {
            if (this.state !== 'configured') return;
            this.decodedFrames = (this.decodedFrames | 0) + 1;
            if (typeof this._output === 'function') {
                this._output(new VideoFrame(null, {
                    codedWidth: 16,
                    codedHeight: 16,
                    timestamp: (chunk && typeof chunk.timestamp === 'number') ? chunk.timestamp : 0
                }));
            }
        }
        flush() {
            return Promise.resolve(undefined);
        }
        reset() {
            this.state = 'unconfigured';
            this.decodeQueueSize = 0;
        }
        close() {
            this.state = 'closed';
        }
        static isConfigSupported(config) {
            return Promise.resolve({ supported: true, config: config || {} });
        }
    }

    // =========================================================================
    // 10. Custom Elements v1 (CustomElementRegistry, HTMLElement, ShadowRoot)
    // =========================================================================
    function Node() {}
    Node.ELEMENT_NODE = 1;
    Node.ATTRIBUTE_NODE = 2;
    Node.TEXT_NODE = 3;
    Node.COMMENT_NODE = 8;
    Node.DOCUMENT_NODE = 9;
    Node.DOCUMENT_TYPE_NODE = 10;
    Node.DOCUMENT_FRAGMENT_NODE = 11;
    if (elProto) {
        Node.prototype = elProto;
        elProto.ELEMENT_NODE = 1;
        elProto.TEXT_NODE = 3;
        elProto.COMMENT_NODE = 8;
        elProto.DOCUMENT_NODE = 9;
        elProto.DOCUMENT_FRAGMENT_NODE = 11;
    }

    function Element() {}
    if (elProto) Element.prototype = elProto;

    function HTMLElement() {}
    if (elProto) HTMLElement.prototype = elProto;

    function ShadowRoot() {}
    if (elProto) ShadowRoot.prototype = elProto;

    function DocumentFragment() {}
    if (elProto) DocumentFragment.prototype = elProto;

    function HTMLSlotElement() {}
    if (elProto) HTMLSlotElement.prototype = elProto;

    var nativeCeDefine = (globalObj.customElements && typeof globalObj.customElements.define === 'function')
        ? globalObj.customElements.define.bind(globalObj.customElements)
        : null;

    function isValidCustomElementName(name) {
        if (typeof name !== 'string') return false;
        var s = name.trim();
        if (s.length < 3 || s.indexOf('-') <= 0) return false;
        if (s !== s.toLowerCase()) return false;
        var first = s.charCodeAt(0);
        if (first < 97 || first > 122) return false;
        var reserved = [
            'annotation-xml',
            'color-profile',
            'font-face',
            'font-face-src',
            'font-face-uri',
            'font-face-format',
            'font-face-name',
            'missing-glyph'
        ];
        return reserved.indexOf(s) === -1;
    }

    class CustomElementRegistry {
        constructor() {
            this._defs = {};
            this._ctorToName = [];
            this._whenDefined = {};
        }
        define(name, constructor, options) {
            var tag = String(name || '').trim().toLowerCase();
            if (!isValidCustomElementName(tag)) {
                throw new SyntaxError("Failed to execute 'define' on 'CustomElementRegistry': '" + name + "' is not a valid custom element name");
            }
            if (this._defs[tag]) {
                throw new Error("Failed to execute 'define' on 'CustomElementRegistry': the name '" + tag + "' has already been used with this registry");
            }
            if (typeof constructor !== 'function') {
                throw new TypeError("Failed to execute 'define' on 'CustomElementRegistry': parameter 2 is not of type 'Function'");
            }
            if (nativeCeDefine) {
                nativeCeDefine(tag);
            }
            var observed = [];
            try {
                var rawObs = constructor.observedAttributes;
                if (Array.isArray(rawObs)) {
                    for (var i = 0; i < rawObs.length; i++) {
                        observed.push(String(rawObs[i] || '').toLowerCase());
                    }
                }
            } catch (_e) {}
            var entry = {
                name: tag,
                ctor: constructor,
                observed: observed,
                extendsTag: (options && options.extends) ? String(options.extends).toLowerCase() : null
            };
            this._defs[tag] = entry;
            this._ctorToName.push({ ctor: constructor, name: tag });

            if (typeof document !== 'undefined' && typeof document.querySelectorAll === 'function') {
                var existing = document.querySelectorAll(tag);
                for (var j = 0; j < existing.length; j++) {
                    this._upgradeSingle(existing[j], entry, true);
                }
            }

            if (this._whenDefined[tag]) {
                var resolvers = this._whenDefined[tag].slice();
                delete this._whenDefined[tag];
                for (var k = 0; k < resolvers.length; k++) {
                    resolvers[k](constructor);
                }
            }
        }
        get(name) {
            var tag = String(name || '').trim().toLowerCase();
            var entry = this._defs[tag];
            return entry ? entry.ctor : undefined;
        }
        getName(constructor) {
            for (var i = 0; i < this._ctorToName.length; i++) {
                if (this._ctorToName[i].ctor === constructor) {
                    return this._ctorToName[i].name;
                }
            }
            return null;
        }
        whenDefined(name) {
            var tag = String(name || '').trim().toLowerCase();
            if (!isValidCustomElementName(tag)) {
                return Promise.reject(new SyntaxError("'" + name + "' is not a valid custom element name"));
            }
            if (this._defs[tag]) {
                return Promise.resolve(this._defs[tag].ctor);
            }
            var self = this;
            return new Promise(function(resolve) {
                if (!self._whenDefined[tag]) self._whenDefined[tag] = [];
                self._whenDefined[tag].push(resolve);
            });
        }
        upgrade(root) {
            if (!root) return;
            this._upgradeTree(root, Boolean(root.isConnected));
        }
        _upgradeSingle(el, entry, checkConnected) {
            if (!el || el._ceUpgraded) return;
            var def = entry;
            if (!def) {
                var t = (el.tagName || '').toLowerCase();
                def = this._defs[t];
            }
            if (!def) return;
            el._ceUpgraded = true;
            el._ceDef = def;
            try {
                if (def.ctor && def.ctor.prototype) {
                    if (elProto && Object.getPrototypeOf(def.ctor.prototype) === Object.prototype && def.ctor.prototype !== elProto) {
                        Object.setPrototypeOf(def.ctor.prototype, elProto);
                    }
                    Object.setPrototypeOf(el, def.ctor.prototype);
                }
                def.ctor.call(el);
            } catch (err) {
                dispatchGlobalError(err && err.message ? err.message : String(err), '', 1, 1, err);
            }
            if (def.observed.length > 0 && typeof el.attributeChangedCallback === 'function' && typeof el.getAttributeNames === 'function') {
                var attrNames = el.getAttributeNames();
                for (var i = 0; i < attrNames.length; i++) {
                    var aname = String(attrNames[i] || '').toLowerCase();
                    if (def.observed.indexOf(aname) !== -1) {
                        var aval = el.getAttribute(aname);
                        try {
                            el.attributeChangedCallback(aname, null, aval, null);
                        } catch (err2) {
                            dispatchGlobalError(err2 && err2.message ? err2.message : String(err2), '', 1, 1, err2);
                        }
                    }
                }
            }
            if (checkConnected && el.isConnected && !el._ceConnected) {
                el._ceConnected = true;
                if (typeof el.connectedCallback === 'function') {
                    try {
                        el.connectedCallback();
                    } catch (err3) {
                        dispatchGlobalError(err3 && err3.message ? err3.message : String(err3), '', 1, 1, err3);
                    }
                }
            }
        }
        _upgradeTree(node, isConnectedNow) {
            if (!node) return;
            if (node.nodeType === 1) {
                var tag = (node.tagName || '').toLowerCase();
                if (this._defs[tag]) {
                    this._upgradeSingle(node, this._defs[tag], isConnectedNow);
                }
                if (node._ceUpgraded) {
                    if (isConnectedNow && !node._ceConnected) {
                        node._ceConnected = true;
                        if (typeof node.connectedCallback === 'function') {
                            try {
                                node.connectedCallback();
                            } catch (err) {
                                dispatchGlobalError(err && err.message ? err.message : String(err), '', 1, 1, err);
                            }
                        }
                    } else if (!isConnectedNow && node._ceConnected) {
                        node._ceConnected = false;
                        if (typeof node.disconnectedCallback === 'function') {
                            try {
                                node.disconnectedCallback();
                            } catch (err2) {
                                dispatchGlobalError(err2 && err2.message ? err2.message : String(err2), '', 1, 1, err2);
                            }
                        }
                    }
                }
            }
            var kids = node.children || node.childNodes;
            if (kids && kids.length) {
                for (var i = 0; i < kids.length; i++) {
                    this._upgradeTree(kids[i], isConnectedNow);
                }
            }
        }
    }

    var customElementsRegistry = new CustomElementRegistry();
    globalObj.CustomElementRegistry = CustomElementRegistry;
    globalObj.customElements = customElementsRegistry;

    if (typeof document !== 'undefined' && typeof document.createElement === 'function') {
        var origDocCreateElement = document.createElement.bind(document);
        document.createElement = function(tagName, options) {
            var el = origDocCreateElement(tagName, options);
            if (el) {
                var tag = String(tagName || '').trim().toLowerCase();
                var def = customElementsRegistry._defs[tag];
                if (def) {
                    customElementsRegistry._upgradeSingle(el, def, false);
                }
            }
            return el;
        };
    }

    globalObj.__onCustomElementAttributeChanged = function(target, name, oldVal, newVal) {
        if (!target || oldVal === newVal) return;
        if (!target._ceUpgraded) {
            var tag = (target.tagName || '').toLowerCase();
            if (customElementsRegistry._defs[tag]) {
                customElementsRegistry._upgradeSingle(target, customElementsRegistry._defs[tag], Boolean(target.isConnected));
                return;
            }
        }
        var def = target._ceDef;
        if (!def || !def.observed || def.observed.length === 0) return;
        var lowName = String(name || '').toLowerCase();
        if (def.observed.indexOf(lowName) !== -1 && typeof target.attributeChangedCallback === 'function') {
            try {
                target.attributeChangedCallback(lowName, oldVal, newVal, null);
            } catch (err) {
                dispatchGlobalError(err && err.message ? err.message : String(err), '', 1, 1, err);
            }
        }
    };

    globalObj.__onCustomElementChildListChanged = function(target, addedNodes, removedNodes) {
        var connected = Boolean(target && target.isConnected);
        if (Array.isArray(removedNodes)) {
            for (var i = 0; i < removedNodes.length; i++) {
                customElementsRegistry._upgradeTree(removedNodes[i], false);
            }
        }
        if (Array.isArray(addedNodes)) {
            for (var j = 0; j < addedNodes.length; j++) {
                customElementsRegistry._upgradeTree(addedNodes[j], connected);
            }
        }
    };

    globalObj.__onCustomElementAdopted = function(node, oldDoc, newDoc) {
        function walkAdopted(n) {
            if (!n) return;
            if (n._ceUpgraded && typeof n.adoptedCallback === 'function') {
                try {
                    n.adoptedCallback(oldDoc, newDoc);
                } catch (err) {
                    dispatchGlobalError(err && err.message ? err.message : String(err), '', 1, 1, err);
                }
            }
            var kids = n.children;
            if (kids && kids.length) {
                for (var i = 0; i < kids.length; i++) walkAdopted(kids[i]);
            }
        }
        walkAdopted(node);
    };

    class MessageEvent extends Event {
        constructor(type, init) {
            super(type, init);
            init = init || {};
            this.data = init.data !== undefined ? init.data : null;
            this.origin = init.origin !== undefined ? String(init.origin) : '';
            this.lastEventId = init.lastEventId !== undefined ? String(init.lastEventId) : '';
            this.source = init.source !== undefined ? init.source : null;
            this.ports = Array.isArray(init.ports) ? init.ports : [];
        }
    }

    class CloseEvent extends Event {
        constructor(type, init) {
            super(type, init);
            init = init || {};
            this.wasClean = Boolean(init.wasClean);
            this.code = init.code !== undefined ? Number(init.code) : 1005;
            this.reason = init.reason !== undefined ? String(init.reason) : '';
        }
    }

    class WebSocket extends EventTarget {
        constructor(url, protocols) {
            super();
            var baseHref = (globalObj.location && globalObj.location.href) || 'https://localhost/';
            var resolved = '';
            try {
                var parsed = new URL(String(url), baseHref);
                if (parsed.protocol !== 'ws:' && parsed.protocol !== 'wss:') {
                    throw new Error('SyntaxError: WebSocket scheme must be ws or wss.');
                }
                resolved = parsed.href;
            } catch (e) {
                throw e;
            }
            var protoList = [];
            if (protocols !== undefined) {
                var raw = Array.isArray(protocols) ? protocols : [protocols];
                var seen = {};
                for (var i = 0; i < raw.length; i++) {
                    var p = String(raw[i] || '').trim();
                    if (!p || seen[p]) {
                        throw new Error('SyntaxError: invalid or duplicate WebSocket subprotocol.');
                    }
                    seen[p] = true;
                    protoList.push(p);
                }
            }
            this.url = resolved;
            this.readyState = 0;
            this.protocol = '';
            this.extensions = '';
            this.bufferedAmount = 0;
            this.binaryType = 'blob';
            this.onopen = null;
            this.onmessage = null;
            this.onerror = null;
            this.onclose = null;
            this._nativeId = 0;
            this._pollTimer = 0;

            var connectRes = null;
            if (typeof globalObj.__nativeWebSocketConnect === 'function') {
                try {
                    connectRes = globalObj.__nativeWebSocketConnect(resolved, JSON.stringify(protoList));
                } catch (err) {
                    connectRes = { ok: false, error: String(err && err.message ? err.message : err) };
                }
            } else {
                connectRes = { ok: false, error: 'WebSocket transport unavailable' };
            }

            var self = this;
            queueMicrotask(function() {
                if (self.readyState >= 2 && !(connectRes && connectRes.ok)) {
                    return;
                }
                if (connectRes && connectRes.ok) {
                    self._nativeId = connectRes.id | 0;
                    self.protocol = connectRes.protocol ? String(connectRes.protocol) : '';
                    if (self.readyState === 0) {
                        self.readyState = 1;
                        self._fireHandler('open', new Event('open'));
                        self._startPolling();
                        self._pollOnce();
                    }
                } else {
                    self.readyState = 3;
                    var errMsg = (connectRes && connectRes.error) ? String(connectRes.error) : 'WebSocket connection failed';
                    self._fireHandler('error', new Event('error'));
                    self._fireHandler('close', new CloseEvent('close', {
                        wasClean: false,
                        code: 1006,
                        reason: errMsg
                    }));
                }
            });
        }

        _fireHandler(name, ev) {
            this.dispatchEvent(ev);
        }

        _startPolling() {
            if (this._pollTimer || this.readyState === 3 || typeof setInterval !== 'function') {
                return;
            }
            var self = this;
            this._pollTimer = setInterval(function() {
                self._pollOnce();
            }, 10);
        }

        _stopPolling() {
            if (this._pollTimer && typeof clearInterval === 'function') {
                clearInterval(this._pollTimer);
                this._pollTimer = 0;
            }
        }

        _pollOnce() {
            if (!this._nativeId || typeof globalObj.__nativeWebSocketPoll !== 'function') {
                return;
            }
            var snap = globalObj.__nativeWebSocketPoll(this._nativeId);
            if (!snap || typeof snap !== 'object') return;
            var evs = Array.isArray(snap.events) ? snap.events : [];
            for (var i = 0; i < evs.length; i++) {
                var item = evs[i];
                if (!item || typeof item !== 'object') continue;
                if (item.kind === 'text') {
                    this._fireHandler('message', new MessageEvent('message', {
                        data: String(item.data !== undefined ? item.data : ''),
                        origin: this.url
                    }));
                } else if (item.kind === 'binary') {
                    var bytes = Array.isArray(item.data) ? item.data : [];
                    var u8 = new Uint8Array(bytes);
                    var payload = this.binaryType === 'arraybuffer'
                        ? u8.buffer
                        : new Blob([u8]);
                    this._fireHandler('message', new MessageEvent('message', {
                        data: payload,
                        origin: this.url
                    }));
                } else if (item.kind === 'error') {
                    this._fireHandler('error', new Event('error'));
                } else if (item.kind === 'closed') {
                    this.readyState = 3;
                    this._stopPolling();
                    var code = item.code !== undefined ? Number(item.code) : 1005;
                    var reason = item.reason !== undefined ? String(item.reason) : '';
                    this._fireHandler('close', new CloseEvent('close', {
                        wasClean: code === 1000,
                        code: code,
                        reason: reason
                    }));
                }
            }
            if ((snap.readyState | 0) === 3 && this.readyState !== 3) {
                this.readyState = 3;
                this._stopPolling();
            }
        }

        send(data) {
            if (this.readyState === 0) {
                throw new Error('InvalidStateError: WebSocket is still CONNECTING.');
            }
            if (this.readyState !== 1) {
                return;
            }
            if (!this._nativeId || typeof globalObj.__nativeWebSocketSend !== 'function') {
                return;
            }
            var res = null;
            if (data instanceof ArrayBuffer) {
                var arrA = [];
                var uA = new Uint8Array(data);
                for (var ia = 0; ia < uA.length; ia++) arrA.push(uA[ia] & 0xFF);
                res = globalObj.__nativeWebSocketSend(this._nativeId, true, arrA);
            } else if (data && typeof data.byteLength === 'number' && data.buffer) {
                var arrV = [];
                var srcLen = typeof data.length === 'number' ? data.length : (data.byteLength | 0);
                for (var iv = 0; iv < srcLen; iv++) arrV.push(data[iv] & 0xFF);
                res = globalObj.__nativeWebSocketSend(this._nativeId, true, arrV);
            } else if (data instanceof Blob && data._bytes) {
                var arrB = [];
                for (var ib = 0; ib < data._bytes.length; ib++) arrB.push(data._bytes[ib] & 0xFF);
                res = globalObj.__nativeWebSocketSend(this._nativeId, true, arrB);
            } else {
                res = globalObj.__nativeWebSocketSend(this._nativeId, false, String(data));
            }
            if (res && res.ok === false) {
                this._fireHandler('error', new Event('error'));
            }
        }

        close(code, reason) {
            if (code !== undefined) {
                code = Number(code) | 0;
                if (code !== 1000 && (code < 3000 || code > 4999)) {
                    throw new Error('InvalidAccessError: close code must be 1000 or 3000-4999.');
                }
            } else {
                code = 1000;
            }
            var cleanReason = reason !== undefined ? String(reason) : '';
            if (this.readyState === 2 || this.readyState === 3) {
                return;
            }
            if (this.readyState === 0) {
                this.readyState = 3;
                this._stopPolling();
                var self = this;
                queueMicrotask(function() {
                    self._fireHandler('close', new CloseEvent('close', {
                        wasClean: false,
                        code: 1006,
                        reason: cleanReason
                    }));
                });
                return;
            }
            this.readyState = 2;
            if (this._nativeId && typeof globalObj.__nativeWebSocketClose === 'function') {
                globalObj.__nativeWebSocketClose(this._nativeId, code, cleanReason);
            }
            this._pollOnce();
        }
    }
    WebSocket.CONNECTING = 0;
    WebSocket.OPEN = 1;
    WebSocket.CLOSING = 2;
    WebSocket.CLOSED = 3;
    WebSocket.prototype.CONNECTING = 0;
    WebSocket.prototype.OPEN = 1;
    WebSocket.prototype.CLOSING = 2;
    WebSocket.prototype.CLOSED = 3;

    function cloneMessageData(v) {
        if (v === undefined || v === null || typeof v !== 'object') return v;
        if (typeof globalObj.structuredClone === 'function') {
            try {
                return globalObj.structuredClone(v);
            } catch (_e) {}
        }
        try {
            return JSON.parse(JSON.stringify(v));
        } catch (_e2) {
            return v;
        }
    }

    function extractTransferPorts(transfer) {
        var ports = [];
        if (Array.isArray(transfer)) {
            for (var i = 0; i < transfer.length; i++) {
                if (transfer[i] instanceof MessagePort) {
                    ports.push(transfer[i]);
                }
            }
        }
        return ports;
    }

    class MessagePort extends EventTarget {
        constructor() {
            super();
            this._entangledPort = null;
            this._closed = false;
            this._started = false;
            this._queue = [];
            this._onmessage = null;
            this.onmessageerror = null;
        }
        get onmessage() {
            return this._onmessage;
        }
        set onmessage(fn) {
            this._onmessage = typeof fn === 'function' ? fn : null;
            this.start();
        }
        addEventListener(type, callback, options) {
            super.addEventListener(type, callback, options);
            if (String(type || '').toLowerCase() === 'message') {
                this.start();
            }
        }
        start() {
            if (this._closed) return;
            this._started = true;
            this._flushQueue();
        }
        _flushQueue() {
            if (!this._started || this._closed || this._queue.length === 0) return;
            var self = this;
            queueMicrotask(function() {
                if (self._closed || !self._started) return;
                while (self._queue.length > 0 && !self._closed) {
                    var item = self._queue.shift();
                    var ev = new MessageEvent('message', {
                        data: item.data,
                        ports: item.ports
                    });
                    self.dispatchEvent(ev);
                }
            });
        }
        postMessage(message, transfer) {
            if (this._closed) return;
            var target = this._entangledPort;
            if (!target || target._closed) return;
            var cloned = cloneMessageData(message);
            var ports = extractTransferPorts(transfer);
            target._queue.push({ data: cloned, ports: ports });
            target._flushQueue();
        }
        close() {
            this._closed = true;
            this._queue = [];
        }
    }

    class MessageChannel {
        constructor() {
            this.port1 = new MessagePort();
            this.port2 = new MessagePort();
            this.port1._entangledPort = this.port2;
            this.port2._entangledPort = this.port1;
        }
    }

    var _broadcastChannels = [];

    class BroadcastChannel extends EventTarget {
        constructor(name) {
            super();
            this.name = String(name === undefined ? '' : name);
            this.onmessage = null;
            this.onmessageerror = null;
            this._closed = false;
            this._origin = (globalObj.location && globalObj.location.origin) || 'https://localhost';
            _broadcastChannels.push(this);
        }
        postMessage(message) {
            if (this._closed) {
                var err = new Error("InvalidStateError: Failed to execute 'postMessage' on 'BroadcastChannel': Channel is closed");
                err.name = 'InvalidStateError';
                throw err;
            }
            var cloned = cloneMessageData(message);
            var sender = this;
            var targets = [];
            for (var i = 0; i < _broadcastChannels.length; i++) {
                var ch = _broadcastChannels[i];
                if (ch !== sender && !ch._closed && ch.name === sender.name && ch._origin === sender._origin) {
                    targets.push(ch);
                }
            }
            queueMicrotask(function() {
                for (var j = 0; j < targets.length; j++) {
                    var tgt = targets[j];
                    if (tgt._closed) continue;
                    var ev = new MessageEvent('message', {
                        data: cloneMessageData(cloned),
                        origin: sender._origin
                    });
                    tgt.dispatchEvent(ev);
                }
            });
        }
        close() {
            this._closed = true;
            var idx = _broadcastChannels.indexOf(this);
            if (idx !== -1) _broadcastChannels.splice(idx, 1);
        }
    }

    function compileWorkerFunction(source) {
        if (typeof globalObj.__nativeCompileWorkerFn === 'function') {
            return globalObj.__nativeCompileWorkerFn(String(source || ''));
        }
        var wrapped = '(function(self, postMessage, addEventListener, removeEventListener, dispatchEvent, close, importScripts, location, navigator, performance, crypto, indexedDB, MessageChannel, MessagePort, BroadcastChannel, Worker, CustomEvent, Event, MessageEvent, ErrorEvent) {\n' +
            'var onmessage = null, onerror = null;\n' +
            'self.__syncGlobals = function() {\n' +
            '  if (typeof onmessage === "function" && self.onmessage !== onmessage) self.onmessage = onmessage;\n' +
            '  if (typeof onerror === "function" && self.onerror !== onerror) self.onerror = onerror;\n' +
            '};\n' +
            String(source || '') +
            '\n;self.__syncGlobals();\n})';
        return globalObj.eval(wrapped);
    }

    function resolveWorkerSource(rawUrl, baseHref, baseOrigin) {
        if (rawUrl.indexOf('blob:') === 0) {
            var afterBlob = rawUrl.slice(5);
            if (afterBlob.indexOf('http://') === 0 || afterBlob.indexOf('https://') === 0) {
                var blobParsed = new URL(afterBlob, baseHref);
                if (blobParsed.origin !== baseOrigin) {
                    var secErrB = new Error("SecurityError: Failed to construct 'Worker': Script at '" + rawUrl + "' cannot be accessed from origin '" + baseOrigin + "'.");
                    secErrB.name = 'SecurityError';
                    throw secErrB;
                }
            }
            var blobObj = URL._resolveBlobUrl(rawUrl);
            if (blobObj && typeof blobObj._text === 'string') {
                return { ok: true, code: blobObj._text, href: rawUrl };
            }
            return { ok: false, error: 'Failed to load blob Worker script: ' + rawUrl, href: rawUrl };
        }
        if (rawUrl.indexOf('data:') === 0) {
            var commaIdx = rawUrl.indexOf(',');
            if (commaIdx === -1) {
                return { ok: false, error: 'Invalid data: URL for Worker', href: rawUrl };
            }
            var header = rawUrl.slice(5, commaIdx).toLowerCase();
            var payload = rawUrl.slice(commaIdx + 1);
            var decoded = '';
            try {
                if (header.indexOf(';base64') !== -1 && typeof atob === 'function') {
                    decoded = atob(payload);
                } else {
                    decoded = decodeURIComponent(payload);
                }
                return { ok: true, code: decoded, href: rawUrl };
            } catch (e) {
                return { ok: false, error: String(e && e.message ? e.message : e), href: rawUrl };
            }
        }
        var parsed = new URL(rawUrl, baseHref);
        if (parsed.origin !== baseOrigin) {
            var secErr = new Error("SecurityError: Failed to construct 'Worker': Script at '" + parsed.href + "' cannot be accessed from origin '" + baseOrigin + "'.");
            secErr.name = 'SecurityError';
            throw secErr;
        }
        if (typeof globalObj.__nativeFetch === 'function') {
            var fetched = globalObj.__nativeFetch(parsed.href, 'GET', '', '', '', false, 'same-origin');
            if (fetched && fetched.status >= 200 && fetched.status < 300 && typeof fetched.body === 'string') {
                return { ok: true, code: fetched.body, href: parsed.href };
            }
            return {
                ok: false,
                error: 'Failed to load Worker script (' + (fetched ? fetched.status : 0) + '): ' + parsed.href,
                href: parsed.href
            };
        }
        return { ok: false, error: 'Worker script fetch unavailable: ' + parsed.href, href: parsed.href };
    }

    class Worker extends EventTarget {
        constructor(scriptURL, options) {
            super();
            var rawUrl = String(scriptURL === undefined ? '' : scriptURL).trim();
            var baseHref = (globalObj.location && globalObj.location.href) || 'https://localhost/';
            var baseOrigin = (globalObj.location && globalObj.location.origin) || (new URL(baseHref)).origin;
            var resolved = resolveWorkerSource(rawUrl, baseHref, baseOrigin);

            this.onmessage = null;
            this.onmessageerror = null;
            this.onerror = null;
            this._terminated = false;
            this._ready = false;
            this._inQueue = [];
            this._options = options || {};

            var selfWorker = this;
            var workerScope = new EventTarget();
            this._scope = workerScope;
            workerScope.self = workerScope;
            workerScope.globalThis = workerScope;
            workerScope.name = (options && options.name !== undefined) ? String(options.name) : '';
            workerScope.onmessage = null;
            workerScope.onmessageerror = null;
            workerScope.onerror = null;
            workerScope._closed = false;
            var locHref = (resolved.href.indexOf('blob:') === 0 || resolved.href.indexOf('data:') === 0)
                ? baseHref
                : resolved.href;
            workerScope.location = new URL(locHref, baseHref);
            workerScope.origin = baseOrigin;
            workerScope.navigator = {
                userAgent: (globalObj.navigator && globalObj.navigator.userAgent) || 'AnvilBrowser/0.28',
                language: (globalObj.navigator && globalObj.navigator.language) || 'en-US',
                languages: ['en-US', 'en'],
                hardwareConcurrency: 4,
                onLine: true
            };
            workerScope.performance = globalObj.performance;
            workerScope.crypto = globalObj.crypto;
            workerScope.indexedDB = globalObj.indexedDB;
            workerScope.MessageChannel = MessageChannel;
            workerScope.MessagePort = MessagePort;
            workerScope.BroadcastChannel = BroadcastChannel;
            workerScope.Worker = Worker;
            workerScope.CustomEvent = CustomEvent;
            workerScope.Event = Event;
            workerScope.MessageEvent = MessageEvent;
            workerScope.ErrorEvent = ErrorEvent;

            workerScope.postMessage = function(data, transfer) {
                if (selfWorker._terminated || workerScope._closed) return;
                var cloned = cloneMessageData(data);
                var ports = extractTransferPorts(transfer);
                queueMicrotask(function() {
                    if (selfWorker._terminated) return;
                    var ev = new MessageEvent('message', {
                        data: cloned,
                        origin: baseOrigin,
                        ports: ports
                    });
                    selfWorker.dispatchEvent(ev);
                });
            };

            workerScope.close = function() {
                workerScope._closed = true;
                selfWorker._terminated = true;
                selfWorker._inQueue = [];
            };

            workerScope.importScripts = function() {
                if (selfWorker._terminated || workerScope._closed) return;
                for (var i = 0; i < arguments.length; i++) {
                    var spec = String(arguments[i] || '').trim();
                    if (!spec) continue;
                    var sub = resolveWorkerSource(spec, workerScope.location.href, baseOrigin);
                    if (!sub.ok) {
                        var netErr = new Error('NetworkError: ' + sub.error);
                        netErr.name = 'NetworkError';
                        throw netErr;
                    }
                    selfWorker._execInWorkerScope(sub.code);
                }
            };

            queueMicrotask(function() {
                if (selfWorker._terminated) return;
                if (!resolved.ok) {
                    selfWorker._reportWorkerError(new Error(resolved.error), resolved.href);
                    return;
                }
                try {
                    selfWorker._execInWorkerScope(resolved.code);
                    selfWorker._ready = true;
                    selfWorker._pumpInQueue();
                } catch (err) {
                    selfWorker._reportWorkerError(err, resolved.href);
                }
            });
        }

        _execInWorkerScope(code) {
            var ws = this._scope;
            var fn = compileWorkerFunction(code);
            fn.call(
                ws,
                ws,
                ws.postMessage,
                ws.addEventListener.bind(ws),
                ws.removeEventListener.bind(ws),
                ws.dispatchEvent.bind(ws),
                ws.close,
                ws.importScripts,
                ws.location,
                ws.navigator,
                ws.performance,
                ws.crypto,
                ws.indexedDB,
                MessageChannel,
                MessagePort,
                BroadcastChannel,
                Worker,
                CustomEvent,
                Event,
                MessageEvent,
                ErrorEvent
            );
            if (typeof ws.__syncGlobals === 'function') {
                ws.__syncGlobals();
            }
        }

        _reportWorkerError(err, filename) {
            if (this._terminated) return;
            var msg = (err && err.message) ? String(err.message) : String(err || 'Worker error');
            var ws = this._scope;
            if (ws && typeof ws.__syncGlobals === 'function') {
                ws.__syncGlobals();
            }
            if (ws && typeof ws.onerror === 'function') {
                try {
                    var handled = ws.onerror(msg, filename || '', 1, 1, err);
                    if (handled === true) return;
                } catch (_e) {}
            }
            var errEv = new ErrorEvent('error', {
                message: msg,
                filename: String(filename || ''),
                lineno: 1,
                colno: 1,
                error: err || new Error(msg),
                cancelable: true
            });
            this.dispatchEvent(errEv);
        }

        _pumpInQueue() {
            if (!this._ready || this._terminated) return;
            var selfWorker = this;
            var ws = this._scope;
            var baseOrigin = (ws && ws.origin) || 'https://localhost';
            while (this._inQueue.length > 0 && !this._terminated) {
                var item = this._inQueue.shift();
                (function(msgItem) {
                    queueMicrotask(function() {
                        if (selfWorker._terminated || !ws || ws._closed) return;
                        if (typeof ws.__syncGlobals === 'function') {
                            ws.__syncGlobals();
                        }
                        var ev = new MessageEvent('message', {
                            data: msgItem.data,
                            origin: baseOrigin,
                            ports: msgItem.ports
                        });
                        ws.dispatchEvent(ev);
                    });
                })(item);
            }
        }

        postMessage(message, transfer) {
            if (this._terminated) return;
            var cloned = cloneMessageData(message);
            var ports = extractTransferPorts(transfer);
            this._inQueue.push({ data: cloned, ports: ports });
            this._pumpInQueue();
        }

        terminate() {
            this._terminated = true;
            this._inQueue = [];
            if (this._scope) {
                this._scope._closed = true;
            }
        }
    }

    function idbDeepClone(v) {
        if (v === undefined || v === null) return v;
        if (typeof v !== 'object') return v;
        return JSON.parse(JSON.stringify(v));
    }

    function idbCompareKeys(a, b) {
        if (a === b) return 0;
        var ta = typeof a;
        var tb = typeof b;
        var rank = function(x) {
            if (typeof x === 'number') return 1;
            if (typeof x === 'string') return 3;
            if (Array.isArray(x)) return 4;
            return 2;
        };
        var ra = rank(a);
        var rb = rank(b);
        if (ra !== rb) return ra < rb ? -1 : 1;
        if (ta === 'number') {
            if (a < b) return -1;
            if (a > b) return 1;
            return 0;
        }
        if (ta === 'string') {
            return a < b ? -1 : (a > b ? 1 : 0);
        }
        if (Array.isArray(a) && Array.isArray(b)) {
            var n = Math.min(a.length, b.length);
            for (var i = 0; i < n; i++) {
                var c = idbCompareKeys(a[i], b[i]);
                if (c !== 0) return c;
            }
            return a.length < b.length ? -1 : (a.length > b.length ? 1 : 0);
        }
        var sa = String(a);
        var sb = String(b);
        return sa < sb ? -1 : (sa > sb ? 1 : 0);
    }

    function idbReadKeyPath(value, keyPath) {
        if (keyPath === null || keyPath === undefined) return undefined;
        if (Array.isArray(keyPath)) {
            var res = [];
            for (var i = 0; i < keyPath.length; i++) {
                var part = idbReadKeyPath(value, keyPath[i]);
                if (part === undefined) return undefined;
                res.push(part);
            }
            return res;
        }
        var parts = String(keyPath).split('.');
        var cur = value;
        for (var j = 0; j < parts.length; j++) {
            if (cur === null || cur === undefined || typeof cur !== 'object') return undefined;
            cur = cur[parts[j]];
        }
        return cur;
    }

    function idbAssignKeyPath(value, keyPath, key) {
        if (typeof keyPath !== 'string' || !value || typeof value !== 'object') return;
        var parts = keyPath.split('.');
        var cur = value;
        for (var i = 0; i < parts.length - 1; i++) {
            var seg = parts[i];
            if (!cur[seg] || typeof cur[seg] !== 'object') cur[seg] = {};
            cur = cur[seg];
        }
        cur[parts[parts.length - 1]] = idbDeepClone(key);
    }

    class IDBKeyRange {
        constructor(lower, upper, lowerOpen, upperOpen) {
            this.lower = lower;
            this.upper = upper;
            this.lowerOpen = Boolean(lowerOpen);
            this.upperOpen = Boolean(upperOpen);
        }

        includes(key) {
            if (this.lower !== undefined) {
                var cmpL = idbCompareKeys(key, this.lower);
                if (cmpL < 0 || (this.lowerOpen && cmpL === 0)) return false;
            }
            if (this.upper !== undefined) {
                var cmpU = idbCompareKeys(key, this.upper);
                if (cmpU > 0 || (this.upperOpen && cmpU === 0)) return false;
            }
            return true;
        }

        static only(value) {
            return new IDBKeyRange(value, value, false, false);
        }
        static lowerBound(lower, open) {
            return new IDBKeyRange(lower, undefined, Boolean(open), false);
        }
        static upperBound(upper, open) {
            return new IDBKeyRange(undefined, upper, false, Boolean(open));
        }
        static bound(lower, upper, lowerOpen, upperOpen) {
            if (idbCompareKeys(lower, upper) > 0) {
                throw new Error('DataError: lower key cannot be greater than upper key.');
            }
            return new IDBKeyRange(lower, upper, Boolean(lowerOpen), Boolean(upperOpen));
        }
    }

    function idbNormalizeRange(query) {
        if (query === undefined || query === null) return null;
        if (query instanceof IDBKeyRange) return query;
        return IDBKeyRange.only(query);
    }

    class IDBVersionChangeEvent extends Event {
        constructor(type, init) {
            super(type, init);
            init = init || {};
            this.oldVersion = init.oldVersion !== undefined ? Number(init.oldVersion) : 0;
            this.newVersion = init.newVersion === null ? null : Number(init.newVersion || 0);
        }
    }

    class IDBRequest extends EventTarget {
        constructor(source, transaction) {
            super();
            this.result = undefined;
            this.error = null;
            this.source = source || null;
            this.transaction = transaction || null;
            this.readyState = 'pending';
            this.onsuccess = null;
            this.onerror = null;
        }

        _emit(type, ev) {
            this.dispatchEvent(ev);
        }

        _resolve(val) {
            this.result = val;
            this.error = null;
            this.readyState = 'done';
            var ev = new Event('success');
            this._emit('success', ev);
        }

        _reject(err) {
            this.error = err instanceof Error ? err : new Error(String(err));
            this.readyState = 'done';
            var ev = new Event('error');
            this._emit('error', ev);
            if (this.transaction) {
                this.transaction._handleError(this.error);
            }
        }
    }

    class IDBOpenDBRequest extends IDBRequest {
        constructor(source) {
            super(source, null);
            this.onupgradeneeded = null;
            this.onblocked = null;
        }
    }

    class IDBCursor {
        constructor(request, source, entries, direction, keyOnly, tx, storeRef) {
            this.request = request;
            this.source = source;
            this.direction = direction || 'next';
            this._entries = entries;
            this._idx = 0;
            this._keyOnly = Boolean(keyOnly);
            this._tx = tx;
            this._storeRef = storeRef;
            this._loadCurrent();
        }

        _loadCurrent() {
            if (this._idx >= this._entries.length) {
                this.key = undefined;
                this.primaryKey = undefined;
                if (!this._keyOnly) this.value = undefined;
                return false;
            }
            var item = this._entries[this._idx];
            this.key = idbDeepClone(item.key);
            this.primaryKey = idbDeepClone(item.primaryKey);
            if (!this._keyOnly) {
                this.value = idbDeepClone(item.value);
            }
            return true;
        }

        continue(key) {
            this._tx._assertActive();
            this._idx += 1;
            if (key !== undefined) {
                var desc = this.direction === 'prev' || this.direction === 'prevunique';
                while (this._idx < this._entries.length) {
                    var cmp = idbCompareKeys(this._entries[this._idx].key, key);
                    if ((!desc && cmp >= 0) || (desc && cmp <= 0)) break;
                    this._idx += 1;
                }
            }
            var self = this;
            this._tx._scheduleOp(this.request, function(req) {
                if (self._loadCurrent()) {
                    req._resolve(self);
                } else {
                    req._resolve(null);
                }
            });
        }

        advance(count) {
            var n = Number(count) | 0;
            if (n <= 0) throw new Error('TypeError: advance count must be positive.');
            this._tx._assertActive();
            this._idx += n;
            var self = this;
            this._tx._scheduleOp(this.request, function(req) {
                if (self._loadCurrent()) {
                    req._resolve(self);
                } else {
                    req._resolve(null);
                }
            });
        }

        update(value) {
            this._tx._assertWritable();
            var pk = this.primaryKey;
            return this._storeRef._enqueue(function(req) {
                var clone = idbDeepClone(value);
                if (this._meta.keyPath !== null && this._meta.keyPath !== undefined) {
                    idbAssignKeyPath(clone, this._meta.keyPath, pk);
                }
                this._upsert(pk, clone, false);
                req._resolve(idbDeepClone(pk));
            }.bind(this._storeRef));
        }

        delete() {
            this._tx._assertWritable();
            var pk = this.primaryKey;
            return this._storeRef.delete(pk);
        }
    }

    class IDBCursorWithValue extends IDBCursor {}

    class IDBIndex {
        constructor(store, name, meta) {
            this.objectStore = store;
            this.name = name;
            this.keyPath = idbDeepClone(meta.keyPath);
            this.unique = Boolean(meta.unique);
            this.multiEntry = Boolean(meta.multiEntry);
            this._meta = meta;
        }

        _collectEntries(query, direction) {
            var dir = direction || 'next';
            var range = idbNormalizeRange(query);
            var out = [];
            var recs = this.objectStore._meta.records;
            for (var i = 0; i < recs.length; i++) {
                var rec = recs[i];
                var extracted = idbReadKeyPath(rec.v, this._meta.keyPath);
                if (extracted === undefined) continue;
                var idxKeys = (this.multiEntry && Array.isArray(extracted)) ? extracted : [extracted];
                for (var j = 0; j < idxKeys.length; j++) {
                    var ik = idxKeys[j];
                    if (ik === undefined || ik === null) continue;
                    if (range && !range.includes(ik)) continue;
                    out.push({ key: ik, primaryKey: rec.k, value: rec.v });
                }
            }
            var desc = dir === 'prev' || dir === 'prevunique';
            out.sort(function(a, b) {
                var c = idbCompareKeys(a.key, b.key);
                if (c === 0) c = idbCompareKeys(a.primaryKey, b.primaryKey);
                return desc ? -c : c;
            });
            if (dir === 'nextunique' || dir === 'prevunique') {
                var uniq = [];
                for (var u = 0; u < out.length; u++) {
                    if (uniq.length === 0 || idbCompareKeys(uniq[uniq.length - 1].key, out[u].key) !== 0) {
                        uniq.push(out[u]);
                    }
                }
                return uniq;
            }
            return out;
        }

        get(query) {
            var self = this;
            return this.objectStore._enqueueForSource(this, function(req) {
                var hits = self._collectEntries(query, 'next');
                req._resolve(hits.length > 0 ? idbDeepClone(hits[0].value) : undefined);
            });
        }

        getKey(query) {
            var self = this;
            return this.objectStore._enqueueForSource(this, function(req) {
                var hits = self._collectEntries(query, 'next');
                req._resolve(hits.length > 0 ? idbDeepClone(hits[0].primaryKey) : undefined);
            });
        }

        getAll(query, count) {
            var self = this;
            return this.objectStore._enqueueForSource(this, function(req) {
                var hits = self._collectEntries(query, 'next');
                var lim = count !== undefined ? Math.max(0, count | 0) : hits.length;
                var res = [];
                for (var i = 0; i < hits.length && i < lim; i++) res.push(idbDeepClone(hits[i].value));
                req._resolve(res);
            });
        }

        getAllKeys(query, count) {
            var self = this;
            return this.objectStore._enqueueForSource(this, function(req) {
                var hits = self._collectEntries(query, 'next');
                var lim = count !== undefined ? Math.max(0, count | 0) : hits.length;
                var res = [];
                for (var i = 0; i < hits.length && i < lim; i++) res.push(idbDeepClone(hits[i].primaryKey));
                req._resolve(res);
            });
        }

        count(query) {
            var self = this;
            return this.objectStore._enqueueForSource(this, function(req) {
                req._resolve(self._collectEntries(query, 'next').length);
            });
        }

        openCursor(query, direction) {
            var self = this;
            return this.objectStore._enqueueForSource(this, function(req) {
                var hits = self._collectEntries(query, direction || 'next');
                if (hits.length === 0) {
                    req._resolve(null);
                } else {
                    req._resolve(new IDBCursorWithValue(req, self, hits, direction || 'next', false, self.objectStore.transaction, self.objectStore));
                }
            });
        }

        openKeyCursor(query, direction) {
            var self = this;
            return this.objectStore._enqueueForSource(this, function(req) {
                var hits = self._collectEntries(query, direction || 'next');
                if (hits.length === 0) {
                    req._resolve(null);
                } else {
                    req._resolve(new IDBCursor(req, self, hits, direction || 'next', true, self.objectStore.transaction, self.objectStore));
                }
            });
        }
    }

    class IDBObjectStore {
        constructor(tx, name, meta) {
            this.transaction = tx;
            this.name = name;
            this._meta = meta;
            this.keyPath = idbDeepClone(meta.keyPath);
            this.autoIncrement = Boolean(meta.autoIncrement);
            this.indexNames = Object.keys(meta.indexes || {}).sort();
        }

        createIndex(name, keyPath, options) {
            this.transaction._assertVersionChange();
            name = String(name);
            if (Object.prototype.hasOwnProperty.call(this._meta.indexes, name)) {
                throw new Error('ConstraintError: index already exists: ' + name);
            }
            options = options || {};
            var meta = {
                keyPath: idbDeepClone(keyPath),
                unique: Boolean(options.unique),
                multiEntry: Boolean(options.multiEntry)
            };
            this._meta.indexes[name] = meta;
            this.indexNames = Object.keys(this._meta.indexes).sort();
            return new IDBIndex(this, name, meta);
        }

        deleteIndex(name) {
            this.transaction._assertVersionChange();
            name = String(name);
            delete this._meta.indexes[name];
            this.indexNames = Object.keys(this._meta.indexes).sort();
        }

        index(name) {
            name = String(name);
            if (!Object.prototype.hasOwnProperty.call(this._meta.indexes, name)) {
                throw new Error('NotFoundError: index not found: ' + name);
            }
            return new IDBIndex(this, name, this._meta.indexes[name]);
        }

        _enqueue(op) {
            return this._enqueueForSource(this, op);
        }

        _enqueueForSource(source, op) {
            this.transaction._assertActive();
            var req = new IDBRequest(source, this.transaction);
            this.transaction._scheduleOp(req, op);
            return req;
        }

        _upsert(key, value, noOverwrite) {
            var recs = this._meta.records;
            var existingIdx = -1;
            for (var i = 0; i < recs.length; i++) {
                if (idbCompareKeys(recs[i].k, key) === 0) {
                    existingIdx = i;
                    break;
                }
            }
            if (noOverwrite && existingIdx !== -1) {
                throw new Error('ConstraintError: key already exists.');
            }
            for (var idxName in this._meta.indexes) {
                if (!Object.prototype.hasOwnProperty.call(this._meta.indexes, idxName)) continue;
                var idxMeta = this._meta.indexes[idxName];
                if (!idxMeta.unique) continue;
                var idxVal = idbReadKeyPath(value, idxMeta.keyPath);
                if (idxVal === undefined || idxVal === null) continue;
                for (var r = 0; r < recs.length; r++) {
                    if (r === existingIdx) continue;
                    var other = idbReadKeyPath(recs[r].v, idxMeta.keyPath);
                    if (other !== undefined && idbCompareKeys(other, idxVal) === 0) {
                        throw new Error('ConstraintError: unique index violation on ' + idxName);
                    }
                }
            }
            var clonedVal = idbDeepClone(value);
            var clonedKey = idbDeepClone(key);
            if (existingIdx !== -1) {
                recs[existingIdx] = { k: clonedKey, v: clonedVal };
            } else {
                recs.push({ k: clonedKey, v: clonedVal });
                recs.sort(function(a, b) { return idbCompareKeys(a.k, b.k); });
            }
            return idbDeepClone(clonedKey);
        }

        _writeValue(value, key, noOverwrite) {
            var val = idbDeepClone(value);
            var resolvedKey = key;
            if (this._meta.keyPath !== null && this._meta.keyPath !== undefined) {
                if (resolvedKey !== undefined) {
                    throw new Error('DataError: out-of-line key disallowed when keyPath is set.');
                }
                resolvedKey = idbReadKeyPath(val, this._meta.keyPath);
                if (resolvedKey === undefined) {
                    if (!this._meta.autoIncrement) {
                        throw new Error('DataError: missing key for keyPath.');
                    }
                    resolvedKey = this._meta.nextKey || 1;
                    this._meta.nextKey = resolvedKey + 1;
                    idbAssignKeyPath(val, this._meta.keyPath, resolvedKey);
                } else if (this._meta.autoIncrement && typeof resolvedKey === 'number' && resolvedKey >= (this._meta.nextKey || 1)) {
                    this._meta.nextKey = Math.floor(resolvedKey) + 1;
                }
            } else if (resolvedKey === undefined) {
                if (!this._meta.autoIncrement) {
                    throw new Error('DataError: key required.');
                }
                resolvedKey = this._meta.nextKey || 1;
                this._meta.nextKey = resolvedKey + 1;
            } else if (this._meta.autoIncrement && typeof resolvedKey === 'number' && resolvedKey >= (this._meta.nextKey || 1)) {
                this._meta.nextKey = Math.floor(resolvedKey) + 1;
            }
            return this._upsert(resolvedKey, val, noOverwrite);
        }

        put(value, key) {
            this.transaction._assertWritable();
            var self = this;
            return this._enqueue(function(req) {
                req._resolve(self._writeValue(value, key, false));
            });
        }

        add(value, key) {
            this.transaction._assertWritable();
            var self = this;
            return this._enqueue(function(req) {
                req._resolve(self._writeValue(value, key, true));
            });
        }

        _matchingRecords(query, direction) {
            var dir = direction || 'next';
            var range = idbNormalizeRange(query);
            var out = [];
            var recs = this._meta.records;
            for (var i = 0; i < recs.length; i++) {
                if (!range || range.includes(recs[i].k)) {
                    out.push({ key: recs[i].k, primaryKey: recs[i].k, value: recs[i].v });
                }
            }
            if (dir === 'prev' || dir === 'prevunique') {
                out.reverse();
            }
            return out;
        }

        get(query) {
            var self = this;
            return this._enqueue(function(req) {
                var hits = self._matchingRecords(query, 'next');
                req._resolve(hits.length > 0 ? idbDeepClone(hits[0].value) : undefined);
            });
        }

        getKey(query) {
            var self = this;
            return this._enqueue(function(req) {
                var hits = self._matchingRecords(query, 'next');
                req._resolve(hits.length > 0 ? idbDeepClone(hits[0].key) : undefined);
            });
        }

        getAll(query, count) {
            var self = this;
            return this._enqueue(function(req) {
                var hits = self._matchingRecords(query, 'next');
                var lim = count !== undefined ? Math.max(0, count | 0) : hits.length;
                var res = [];
                for (var i = 0; i < hits.length && i < lim; i++) res.push(idbDeepClone(hits[i].value));
                req._resolve(res);
            });
        }

        getAllKeys(query, count) {
            var self = this;
            return this._enqueue(function(req) {
                var hits = self._matchingRecords(query, 'next');
                var lim = count !== undefined ? Math.max(0, count | 0) : hits.length;
                var res = [];
                for (var i = 0; i < hits.length && i < lim; i++) res.push(idbDeepClone(hits[i].key));
                req._resolve(res);
            });
        }

        count(query) {
            var self = this;
            return this._enqueue(function(req) {
                req._resolve(self._matchingRecords(query, 'next').length);
            });
        }

        delete(query) {
            this.transaction._assertWritable();
            var self = this;
            return this._enqueue(function(req) {
                var range = idbNormalizeRange(query);
                if (range) {
                    var kept = [];
                    for (var i = 0; i < self._meta.records.length; i++) {
                        if (!range.includes(self._meta.records[i].k)) {
                            kept.push(self._meta.records[i]);
                        }
                    }
                    self._meta.records = kept;
                }
                req._resolve(undefined);
            });
        }

        clear() {
            this.transaction._assertWritable();
            var self = this;
            return this._enqueue(function(req) {
                self._meta.records = [];
                req._resolve(undefined);
            });
        }

        openCursor(query, direction) {
            var self = this;
            return this._enqueue(function(req) {
                var hits = self._matchingRecords(query, direction || 'next');
                if (hits.length === 0) {
                    req._resolve(null);
                } else {
                    req._resolve(new IDBCursorWithValue(req, self, hits, direction || 'next', false, self.transaction, self));
                }
            });
        }

        openKeyCursor(query, direction) {
            var self = this;
            return this._enqueue(function(req) {
                var hits = self._matchingRecords(query, direction || 'next');
                if (hits.length === 0) {
                    req._resolve(null);
                } else {
                    req._resolve(new IDBCursor(req, self, hits, direction || 'next', true, self.transaction, self));
                }
            });
        }
    }

    class IDBTransaction extends EventTarget {
        constructor(db, storeNames, mode) {
            super();
            this.db = db;
            this.objectStoreNames = storeNames.slice();
            this.mode = mode || 'readonly';
            this.error = null;
            this.oncomplete = null;
            this.onabort = null;
            this.onerror = null;
            this._active = true;
            this._finished = false;
            this._aborted = false;
            this._pendingOps = 0;
            this._commitScheduled = false;
            this._snapshot = idbDeepClone(db._state);
        }

        _assertActive() {
            if (!this._active || this._finished) {
                throw new Error('TransactionInactiveError: IndexedDB transaction is not active.');
            }
        }

        _assertWritable() {
            this._assertActive();
            if (this.mode !== 'readwrite' && this.mode !== 'versionchange') {
                throw new Error('ReadOnlyError: transaction mode is readonly.');
            }
        }

        _assertVersionChange() {
            this._assertActive();
            if (this.mode !== 'versionchange') {
                throw new Error('InvalidStateError: allowed only during versionchange.');
            }
        }

        _emitTx(type, ev) {
            this.dispatchEvent(ev);
        }

        objectStore(name) {
            name = String(name);
            if (this.objectStoreNames.indexOf(name) === -1 && this.mode !== 'versionchange') {
                throw new Error('NotFoundError: store not in transaction scope: ' + name);
            }
            var meta = this.db._state.stores[name];
            if (!meta) {
                throw new Error('NotFoundError: objectStore not found: ' + name);
            }
            return new IDBObjectStore(this, name, meta);
        }

        _scheduleOp(req, op) {
            var self = this;
            self._pendingOps += 1;
            queueMicrotask(function() {
                if (self._aborted || self._finished) {
                    self._pendingOps -= 1;
                    return;
                }
                self._active = true;
                try {
                    op(req);
                    if (self.mode !== 'readonly' && typeof globalObj.__nativeIdbSave === 'function') {
                        var saved = globalObj.__nativeIdbSave(self.db.name, JSON.stringify(self.db._state));
                        if (!saved) {
                            throw new Error('QuotaExceededError: IndexedDB origin storage limit reached.');
                        }
                    }
                } catch (err) {
                    req._reject(err);
                } finally {
                    self._pendingOps -= 1;
                    if (!self._aborted && !self._finished && self._pendingOps === 0) {
                        self._scheduleCommit();
                    }
                }
            });
        }

        _scheduleCommit() {
            if (this._commitScheduled || this._finished) return;
            this._commitScheduled = true;
            var self = this;
            queueMicrotask(function() {
                self._commitScheduled = false;
                if (!self._aborted && !self._finished && self._pendingOps === 0) {
                    self.commit();
                }
            });
        }

        _handleError(err) {
            this.error = err;
            this._emitTx('error', new Event('error'));
            if (!this._finished) {
                this.abort();
            }
        }

        abort() {
            if (this._finished) return;
            this._aborted = true;
            this._active = false;
            this._finished = true;
            this.db._state = this._snapshot;
            this.db.version = this.db._state.version;
            this.db.objectStoreNames = Object.keys(this.db._state.stores).sort();
            if (this.mode !== 'readonly' && typeof globalObj.__nativeIdbSave === 'function') {
                globalObj.__nativeIdbSave(this.db.name, JSON.stringify(this.db._state));
            }
            this._emitTx('abort', new Event('abort'));
        }

        commit() {
            if (this._finished || this._aborted) return;
            this._active = false;
            this._finished = true;
            if (this.mode !== 'readonly' && typeof globalObj.__nativeIdbSave === 'function') {
                var saved = globalObj.__nativeIdbSave(this.db.name, JSON.stringify(this.db._state));
                if (!saved) {
                    this.error = new Error('QuotaExceededError: IndexedDB origin storage limit reached.');
                    this._emitTx('abort', new Event('abort'));
                    return;
                }
            }
            this._emitTx('complete', new Event('complete'));
        }
    }

    class IDBDatabase extends EventTarget {
        constructor(name, state) {
            super();
            this.name = String(name);
            this._state = state;
            this.version = state.version;
            this.objectStoreNames = Object.keys(state.stores || {}).sort();
            this.onabort = null;
            this.onclose = null;
            this.onerror = null;
            this.onversionchange = null;
            this._closed = false;
        }

        createObjectStore(name, options) {
            name = String(name);
            if (!this._upgradeTx || this._upgradeTx._finished) {
                throw new Error('InvalidStateError: createObjectStore only valid during onupgradeneeded.');
            }
            if (Object.prototype.hasOwnProperty.call(this._state.stores, name)) {
                throw new Error('ConstraintError: objectStore already exists: ' + name);
            }
            options = options || {};
            var meta = {
                keyPath: options.keyPath !== undefined ? idbDeepClone(options.keyPath) : null,
                autoIncrement: Boolean(options.autoIncrement),
                nextKey: 1,
                records: [],
                indexes: {}
            };
            this._state.stores[name] = meta;
            this.objectStoreNames = Object.keys(this._state.stores).sort();
            if (this._upgradeTx.objectStoreNames.indexOf(name) === -1) {
                this._upgradeTx.objectStoreNames.push(name);
            }
            return new IDBObjectStore(this._upgradeTx, name, meta);
        }

        deleteObjectStore(name) {
            name = String(name);
            if (!this._upgradeTx || this._upgradeTx._finished) {
                throw new Error('InvalidStateError: deleteObjectStore only valid during onupgradeneeded.');
            }
            delete this._state.stores[name];
            this.objectStoreNames = Object.keys(this._state.stores).sort();
        }

        transaction(storeNames, mode) {
            if (this._closed) {
                throw new Error('InvalidStateError: database connection is closing.');
            }
            var list = Array.isArray(storeNames) ? storeNames.slice() : [String(storeNames)];
            for (var i = 0; i < list.length; i++) {
                if (!Object.prototype.hasOwnProperty.call(this._state.stores, list[i])) {
                    throw new Error('NotFoundError: objectStore not found: ' + list[i]);
                }
            }
            var tx = new IDBTransaction(this, list, mode || 'readonly');
            tx._scheduleCommit();
            return tx;
        }

        close() {
            this._closed = true;
        }
    }

    class IDBFactory {
        open(name, version) {
            var dbName = String(name || '');
            if (!dbName) {
                throw new Error('TypeError: IndexedDB database name must be non-empty.');
            }
            var req = new IDBOpenDBRequest(null);
            var reqVersion = version;
            queueMicrotask(function() {
                var raw = typeof globalObj.__nativeIdbLoad === 'function'
                    ? globalObj.__nativeIdbLoad(dbName)
                    : null;
                var existing = null;
                if (raw && typeof raw === 'string') {
                    try { existing = JSON.parse(raw); } catch (_e) { existing = null; }
                }
                var oldVersion = existing ? (existing.version | 0) : 0;
                var targetVersion = reqVersion !== undefined ? (Number(reqVersion) | 0) : (oldVersion > 0 ? oldVersion : 1);
                if (targetVersion <= 0) {
                    req._reject(new Error('TypeError: IndexedDB version must be >= 1.'));
                    return;
                }
                if (oldVersion > 0 && targetVersion < oldVersion) {
                    req._reject(new Error('VersionError: requested version is lower than existing version.'));
                    return;
                }
                var state = existing || { name: dbName, version: targetVersion, stores: {} };
                state.version = targetVersion;
                var db = new IDBDatabase(dbName, state);
                if (targetVersion > oldVersion) {
                    var tx = new IDBTransaction(db, db.objectStoreNames.slice(), 'versionchange');
                    db._upgradeTx = tx;
                    req.transaction = tx;
                    req.result = db;
                    req.readyState = 'done';
                    var upgEv = new IDBVersionChangeEvent('upgradeneeded', {
                        oldVersion: oldVersion,
                        newVersion: targetVersion
                    });
                    req._emit('upgradeneeded', upgEv);
                    db._upgradeTx = null;
                    tx.commit();
                    req.transaction = null;
                    if (!tx._aborted) {
                        req._resolve(db);
                    } else {
                        req._reject(tx.error || new Error('AbortError: versionchange aborted.'));
                    }
                } else {
                    req._resolve(db);
                }
            });
            return req;
        }

        deleteDatabase(name) {
            var dbName = String(name || '');
            var req = new IDBOpenDBRequest(null);
            queueMicrotask(function() {
                if (typeof globalObj.__nativeIdbDelete === 'function') {
                    globalObj.__nativeIdbDelete(dbName);
                }
                req._resolve(undefined);
            });
            return req;
        }

        databases() {
            var list = typeof globalObj.__nativeIdbList === 'function'
                ? globalObj.__nativeIdbList()
                : [];
            return Promise.resolve(Array.isArray(list) ? list : []);
        }

        cmp(a, b) {
            return idbCompareKeys(a, b);
        }
    }

    var indexedDBInstance = new IDBFactory();

    globalObj.EventTarget = EventTarget;
    globalObj.Node = Node;
    globalObj.Element = Element;
    globalObj.HTMLElement = HTMLElement;
    globalObj.ShadowRoot = ShadowRoot;
    globalObj.DocumentFragment = DocumentFragment;
    globalObj.HTMLSlotElement = HTMLSlotElement;
    globalObj.MutationRecord = MutationRecord;
    globalObj.MediaSource = MediaSource;
    globalObj.VideoFrame = VideoFrame;
    globalObj.VideoEncoder = VideoEncoder;
    globalObj.VideoDecoder = VideoDecoder;
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
    globalObj.ReadableStream = ReadableStream;
    globalObj.WritableStream = WritableStream;
    globalObj.TransformStream = TransformStream;
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
    globalObj.File = File;
    globalObj.FileList = FileList;
    globalObj.FileReader = FileReader;
    globalObj.DataTransfer = DataTransfer;
    globalObj.TextEncoder = TextEncoder;
    globalObj.TextDecoder = TextDecoder;
    globalObj.XMLHttpRequest = XMLHttpRequest;
    globalObj.MediaKeys = MediaKeys;
    globalObj.MediaKeySession = MediaKeySession;
    globalObj.MediaKeySystemAccess = MediaKeySystemAccess;
    globalObj.MessageEvent = MessageEvent;
    globalObj.CloseEvent = CloseEvent;
    globalObj.WebSocket = WebSocket;
    globalObj.MessagePort = MessagePort;
    globalObj.MessageChannel = MessageChannel;
    globalObj.BroadcastChannel = BroadcastChannel;
    globalObj.Worker = Worker;
    globalObj.IDBKeyRange = IDBKeyRange;
    globalObj.IDBVersionChangeEvent = IDBVersionChangeEvent;
    globalObj.IDBRequest = IDBRequest;
    globalObj.IDBOpenDBRequest = IDBOpenDBRequest;
    globalObj.IDBCursor = IDBCursor;
    globalObj.IDBCursorWithValue = IDBCursorWithValue;
    globalObj.IDBIndex = IDBIndex;
    globalObj.IDBObjectStore = IDBObjectStore;
    globalObj.IDBTransaction = IDBTransaction;
    globalObj.IDBDatabase = IDBDatabase;
    globalObj.IDBFactory = IDBFactory;
    globalObj.indexedDB = indexedDBInstance;
})();
