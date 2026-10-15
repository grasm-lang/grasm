// Everything on the site that moves.
//
// The rule each piece has to pass: it shows something true that a sentence
// says worse. A rotating object would fail that, so there is not one.
//
//   term()      the compiler refusing a package, typed out at reading speed
//   verdicts()  the same attack against four toolchains, resolved in order
//   engines()   programs entering four engines and leaving as one answer
//   bars()      north-star targets, including the ones still missed
//   lotties()   three loops: the ceiling, the engines, the ownership tiers
//   reveal()    content settling in as it enters view
//
// Everything degrades: with JavaScript off the terminal holds its final text,
// the verdicts show their real states, and the bars sit at full width.

const M = window.Motion || {};
const REDUCED = matchMedia('(prefers-reduced-motion: reduce)').matches;
const sleep = (ms) => new Promise((r) => setTimeout(r, REDUCED ? 0 : ms));

// ------------------------------------------------------------- the terminal

// The real transcript. `scripts/security_bench.sh` produces this every green
// build; it is not written for the page.
const SESSION = [
  { t: '$ ', cls: 'dim', type: 'grasm add mathlib@1.0.1', pause: 380 },
  { t: '  resolved 1 package\n', cls: 'dim', pause: 220 },
  { t: '$ ', cls: 'dim', type: 'grasm build app', pause: 420 },
  { t: 'error', cls: 'err', pause: 0 },
  { t: ': package `mathlib` performs `io` but its manifest only permits nothing\n', pause: 120 },
  { t: '  help: either add the capability to that package’s manifest, or stop\n', cls: 'dim', pause: 60 },
  { t: '        it from performing the effect\n', cls: 'dim', pause: 60 },
  { t: '  performed: `io` at mathlib/src/main.gr:2:9\n\n', cls: 'dim', pause: 260 },
  { t: 'The dependency changed. The build did not ship.', cls: 'ok', pause: 0 },
];

async function term() {
  const body = document.querySelector('.term .body');
  if (!body) return;
  const cursor = document.createElement('span');
  cursor.className = 'cursor';

  async function run() {
    body.textContent = '';
    body.appendChild(cursor);
    for (const step of SESSION) {
      if (step.t) {
        const s = document.createElement('span');
        if (step.cls) s.className = step.cls;
        s.textContent = step.t;
        body.insertBefore(s, cursor);
      }
      if (step.type) {
        // Typed at a readable rate rather than a dramatic one.
        const s = document.createElement('span');
        body.insertBefore(s, cursor);
        for (const ch of step.type) {
          s.textContent += ch;
          await sleep(26);
        }
        const nl = document.createElement('span');
        nl.textContent = '\n';
        body.insertBefore(nl, cursor);
      }
      await sleep(step.pause || 90);
    }
    cursor.remove();
  }

  if (REDUCED) {
    body.textContent = SESSION.map((s) => (s.t || '') + (s.type ? s.type + '\n' : '')).join('');
    return;
  }
  // Only once it is on screen; a terminal that has already finished by the
  // time you scroll to it has shown you nothing.
  if (M.inView) {
    M.inView(body, () => { run(); }, { amount: 0.5 });
  } else {
    run();
  }
}

// ---------------------------------------------------------------- verdicts

async function verdicts() {
  const host = document.querySelector('.verdicts');
  if (!host) return;
  const rows = [...host.querySelectorAll('.verdict-row')];
  const btn = document.querySelector('[data-run-verdicts]');

  const settle = async () => {
    for (const r of rows) {
      r.dataset.state = r.dataset.result;
      if (M.animate && !REDUCED) {
        M.animate(r, { opacity: [0.45, 1] }, { duration: 0.32 });
      }
      await sleep(340);
    }
  };

  if (btn) {
    btn.addEventListener('click', async () => {
      btn.disabled = true;
      rows.forEach((r) => { r.removeAttribute('data-state'); });
      await sleep(240);
      await settle();
      btn.disabled = false;
    });
  }
  if (REDUCED || !M.inView) { rows.forEach((r) => { r.dataset.state = r.dataset.result; }); return; }
  M.inView(host, () => { settle(); }, { amount: 0.4 });
}

// ----------------------------------------------------------- engines canvas

function engines() {
  const cv = document.getElementById('engines');
  if (!cv) return;
  const ctx = cv.getContext('2d');
  if (!ctx) return;

  const NAMES = ['grasm run', 'bundle', '--native', 'wasm32'];
  const total = cv.dataset.programs || '313';
  let w = 0, h = 0;

  const size = () => {
    const r = Math.min(devicePixelRatio, 2);
    w = cv.clientWidth; h = cv.clientHeight;
    cv.width = w * r; cv.height = h * r;
    ctx.setTransform(r, 0, 0, r, 0, 0);
  };
  size();
  addEventListener('resize', size);

  const dots = Array.from({ length: 72 }, (_, i) => ({
    lane: i % 4,
    t: -(i / 72) * 1.5,
    v: 0.17 + (i % 7) * 0.012,
  }));

  const laneY = (i) => 34 + i * ((h - 68) / 3);

  function frame() {
    ctx.clearRect(0, 0, w, h);
    const left = 118, right = w - 210;

    ctx.font = '11px ui-monospace, Menlo, monospace';
    ctx.textBaseline = 'middle';
    for (let i = 0; i < 4; i++) {
      const y = laneY(i);
      ctx.strokeStyle = 'rgba(255,255,255,.14)';
      ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(left, y); ctx.lineTo(right, y); ctx.stroke();
      ctx.fillStyle = '#8c95bf';
      ctx.textAlign = 'right';
      ctx.fillText(NAMES[i], left - 14, y);
    }

    ctx.strokeStyle = 'rgba(10,110,72,.7)';
    ctx.beginPath();
    for (let i = 0; i < 4; i++) { ctx.moveTo(right, laneY(i)); ctx.lineTo(right + 34, h / 2); }
    ctx.stroke();

    for (const d of dots) {
      if (!REDUCED) d.t += d.v * 0.016;
      if (d.t > 1.3) d.t = -0.1;
      if (d.t < 0) continue;
      const p = Math.min(d.t, 1);
      ctx.fillStyle = '#6ee7d8';
      ctx.globalAlpha = p >= 1 ? 1 : 0.8;
      ctx.beginPath();
      ctx.arc(left + (right - left) * p, laneY(d.lane), 2, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
    }

    ctx.fillStyle = '#6ee7d8';
    ctx.beginPath(); ctx.arc(right + 34, h / 2, 4.5, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = '#ffffff';
    ctx.textAlign = 'left';
    ctx.font = '600 12px -apple-system, system-ui, sans-serif';
    ctx.fillText('one answer', right + 46, h / 2 - 7);
    ctx.fillStyle = '#8c95bf';
    ctx.font = '11px ui-monospace, Menlo, monospace';
    ctx.fillText(total + ' programs', right + 46, h / 2 + 9);

    if (!REDUCED) requestAnimationFrame(frame);
  }
  frame();
}

// -------------------------------------------------------------------- bars

function bars() {
  const rows = document.querySelectorAll('.bar-row');
  if (!rows.length) return;
  const show = (row) => {
    const fill = row.querySelector('.bar-fill');
    if (fill) fill.style.width = (fill.dataset.pct || 0) + '%';
  };
  if (M.inView && !REDUCED) rows.forEach((r) => M.inView(r, () => show(r), { amount: 0.5 }));
  else rows.forEach(show);
}

// ------------------------------------------------------------------ lottie

function lotties() {
  const hosts = document.querySelectorAll('[data-lottie]');
  if (!hosts.length || !window.lottie) return;
  hosts.forEach((el) => {
    const anim = window.lottie.loadAnimation({
      container: el,
      renderer: 'svg',
      loop: true,
      autoplay: false,
      path: 'anim/' + el.dataset.lottie + '.json',
    });
    // Played only while visible: three loops running off-screen is three
    // animation frames a second nobody is watching.
    if (REDUCED) { anim.addEventListener('DOMLoaded', () => anim.goToAndStop(45, true)); return; }
    if (M.inView) M.inView(el, () => { anim.play(); return () => anim.pause(); }, { amount: 0.3 });
    else anim.play();
  });
}

// ------------------------------------------------------------------ reveal

function reveal() {
  const items = document.querySelectorAll('.reveal');
  if (!items.length) return;
  if (REDUCED || !M.inView) { items.forEach((el) => el.classList.add('in')); return; }
  items.forEach((el) => M.inView(el, () => el.classList.add('in'), { amount: 0.12 }));
}

// Motion loads with `defer`, so it may not be here yet when this module runs.
function start() {
  term();
  verdicts();
  engines();
  bars();
  lotties();
  reveal();
}
if (document.readyState === 'loading') addEventListener('DOMContentLoaded', start);
else start();
