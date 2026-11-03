// If the app fails before React draws anything (a script that won't load, an
// exception while the bundle starts, a rejected start-up promise), say so
// instead of leaving a blank page. A plain script, loaded before the bundle,
// with no dependencies. Once React has drawn into #root it does nothing.
(function () {
  var shown = false;
  function show(detail) {
    var root = document.getElementById('root');
    if (shown || !root || root.childElementCount > 0) return;
    shown = true;
    var box = document.createElement('div');
    box.setAttribute('role', 'alert');
    box.setAttribute('data-testid', 'boot-error');
    box.style.cssText = 'font: 15px/1.5 system-ui, sans-serif; max-width: 560px; margin: 15vh auto; padding: 0 20px; color: CanvasText;';
    var h = document.createElement('h2');
    h.textContent = "LibreBooks couldn't start";
    var p = document.createElement('p');
    p.textContent = 'Something went wrong while the app was loading. Reloading may help; if it keeps happening, this is a bug worth reporting, with the message below.';
    var pre = document.createElement('pre');
    pre.style.cssText = 'white-space: pre-wrap; font-size: 13px; padding: 10px; border: 1px solid GrayText; border-radius: 4px;';
    pre.textContent = detail || 'Unknown error';
    var again = document.createElement('button');
    again.textContent = 'Reload';
    again.onclick = function () {
      location.reload();
    };
    box.append(h, p, pre, again);
    root.append(box);
  }
  function describe(e) {
    if (!e) return '';
    if (e instanceof Error) return e.name + ': ' + e.message;
    return String(e);
  }
  window.addEventListener(
    'error',
    function (ev) {
      var t = ev.target;
      // A script or stylesheet that didn't load.
      if (t && t !== window && (t.tagName === 'SCRIPT' || t.tagName === 'LINK')) {
        if (t.tagName === 'SCRIPT') show("Couldn't load " + (t.src || 'a script'));
        return;
      }
      show(describe(ev.error) || ev.message);
    },
    true,
  );
  window.addEventListener('unhandledrejection', function (ev) {
    // Only before the first render: later rejections are the app's to show.
    show(describe(ev.reason));
  });
})();
