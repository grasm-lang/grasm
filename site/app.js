// Everything on the site that moves.
//
// Four pieces, each earning its place by showing something that is true and
// hard to say in a sentence:
//
//   stage()      the hero object, built in Blender — a core inside three tier
//                shells inside a ring of capability pillars, one of them cut
//                short because the ceiling is refusable.
//   capability() the supply-chain attack, run against four toolchains. The
//                verdicts are the ones `scripts/security_bench.sh` produces.
//   engines()    313 programs entering four engines and leaving as one answer.
//   bars()       the north-star measurements, including the ones still missed.
//
// Nothing here is decorative-only, and nothing here states a number the
// repository cannot produce. The page degrades to readable text without it.

import * as THREE from './vendor/three.module.js';
import { GLTFLoader } from './vendor/loaders/GLTFLoader.js';

const M = window.Motion || {};
const REDUCED = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

// ---------------------------------------------------------------- hero

function stage() {
  const host = document.getElementById('stage');
  if (!host || !window.WebGLRenderingContext) return;

  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  } catch (_e) {
    return; // No WebGL: the copy beside it still reads.
  }
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.setSize(host.clientWidth, host.clientHeight);
  host.appendChild(renderer.domElement);

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(38, host.clientWidth / host.clientHeight, 0.1, 100);
  camera.position.set(0, 0.35, 7.4);

  // Forest key, brass rim: the two colours the page is built from, so the
  // object belongs to the page rather than sitting on top of it.
  scene.add(new THREE.AmbientLight(0x2b5a45, 1.5));
  const key = new THREE.DirectionalLight(0xd9f0e2, 2.5);
  key.position.set(3.5, 4.5, 5);
  scene.add(key);
  const rim = new THREE.DirectionalLight(0xc89a4e, 3.2);
  rim.position.set(-5, -1.5, -3.5);
  scene.add(rim);
  const fill = new THREE.PointLight(0x066642, 22, 18);
  fill.position.set(-2.5, 2, 3);
  scene.add(fill);

  const root = new THREE.Group();
  scene.add(root);

  new GLTFLoader().load(
    'models/hero.glb',
    (gltf) => {
      const model = gltf.scene;
      // Frame it by its own bounds rather than by a guessed scale, so a
      // re-export from Blender cannot silently change the composition.
      const box = new THREE.Box3().setFromObject(model);
      const size = box.getSize(new THREE.Vector3()).length();
      const mid = box.getCenter(new THREE.Vector3());
      model.position.sub(mid);
      model.scale.setScalar(4.6 / size);
      root.add(model);
      if (M.animate && !REDUCED) {
        M.animate(root.scale, { x: [0.001, 1], y: [0.001, 1], z: [0.001, 1] },
          { duration: 1.1, ease: [0.2, 0.8, 0.25, 1] });
      }
    },
    undefined,
    () => { host.remove(); }   // Model missing: drop the stage, keep the page.
  );

  let mx = 0, my = 0, tx = 0, ty = 0;
  addEventListener('pointermove', (e) => {
    tx = (e.clientX / innerWidth - 0.5) * 0.5;
    ty = (e.clientY / innerHeight - 0.5) * 0.34;
  }, { passive: true });

  addEventListener('resize', () => {
    if (!host.clientWidth) return;
    camera.aspect = host.clientWidth / host.clientHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(host.clientWidth, host.clientHeight);
  });

  const clock = new THREE.Clock();
  renderer.setAnimationLoop(() => {
    const t = clock.getElapsedTime();
    mx += (tx - mx) * 0.045;
    my += (ty - my) * 0.045;
    root.rotation.y = t * (REDUCED ? 0 : 0.13) + mx;
    root.rotation.x = Math.sin(t * 0.22) * 0.08 + my;
    renderer.render(scene, camera);
  });
}

// -------------------------------------------------- the supply-chain attack

// What each toolchain does when a trusted pure library's patch release starts
// reading a file. These are `scripts/security_bench.sh`'s verdicts.
const LANES = [
  { id: 'grasm',  name: 'Grasm',  verdict: 'COMPILE ERROR',
    why: 'the dependency’s inferred effects (io) exceed its declared ceiling ([])',
    state: 'refused' },
  { id: 'rust',   name: 'Rust',   verdict: 'builds clean',
    why: 'std::fs is ambient; no capability is recorded anywhere', state: 'clean' },
  { id: 'go',     name: 'Go',     verdict: 'builds clean',
    why: 'os.ReadFile is importable by any package', state: 'clean' },
  { id: 'python', name: 'Python', verdict: 'compiles clean',
    why: 'open() needs no import and no declaration', state: 'clean' },
];

const TRANSCRIPT = [
  ['dim', '$ mathlib 1.0.0 → 1.0.1   (same signature, same result)'],
  ['dim', '$ grasm build app'],
  ['err', 'error: package `mathlib` performs `io` but its manifest only permits nothing'],
  ['dim', '  help: either add the capability to that package’s manifest, or stop'],
  ['dim', '        it from performing the effect'],
  ['dim', '  performed: `io` at mathlib/src/main.gr:2:9'],
  ['good', '\nThe build fails. Nothing ships.'],
];

function capability() {
  const demo = document.getElementById('capability');
  if (!demo) return;
  const lanes = demo.querySelector('.lanes');
  const term = demo.querySelector('.terminal');
  const btn = demo.querySelector('button');
  if (!lanes || !term || !btn) return;

  lanes.innerHTML = LANES.map((l) => `
    <div class="lane" data-id="${l.id}">
      <div class="name">${l.name}</div>
      <div class="verdict">waiting…</div>
      <div class="why"></div>
    </div>`).join('');

  let running = false;
  btn.addEventListener('click', async () => {
    if (running) return;
    running = true;
    btn.disabled = true;
    btn.textContent = 'shipping v1.0.1…';
    term.innerHTML = '';
    lanes.querySelectorAll('.lane').forEach((el) => {
      el.removeAttribute('data-state');
      el.querySelector('.verdict').textContent = 'building…';
      el.querySelector('.why').textContent = '';
    });

    const wait = (ms) => new Promise((r) => setTimeout(r, REDUCED ? 0 : ms));

    for (const l of LANES) {
      await wait(430);
      const el = lanes.querySelector(`[data-id="${l.id}"]`);
      el.dataset.state = l.state;
      el.querySelector('.verdict').textContent = l.verdict;
      el.querySelector('.why').textContent = l.why;
      if (M.animate && !REDUCED) {
        M.animate(el, { transform: ['scale(0.985)', 'scale(1)'] }, { duration: 0.35 });
      }
    }

    await wait(320);
    for (const [cls, line] of TRANSCRIPT) {
      const span = document.createElement('span');
      span.className = cls;
      span.textContent = line + '\n';
      term.appendChild(span);
      await wait(210);
    }

    btn.disabled = false;
    btn.textContent = 'run it again';
    running = false;
  });
}

// ------------------------------------------------------------ four engines

function engines() {
  const cv = document.getElementById('engines');
  if (!cv) return;
  const ctx = cv.getContext('2d');
  if (!ctx) return;

  const NAMES = ['interpreter', 'bundle', 'native', 'wasm'];
  const total = Number(cv.dataset.programs || 313);
  let w = 0, h = 0;

  function size() {
    const r = Math.min(devicePixelRatio, 2);
    w = cv.clientWidth; h = cv.clientHeight;
    cv.width = w * r; cv.height = h * r;
    ctx.setTransform(r, 0, 0, r, 0, 0);
  }
  size();
  addEventListener('resize', size);

  // One dot per program, staggered so the flow reads as a stream rather than
  // a block. Each crosses a lane and converges on a single answer.
  const dots = Array.from({ length: 96 }, (_, i) => ({
    lane: i % 4,
    t: -(i / 96) * 1.4 - Math.random() * 0.5,
    v: 0.16 + Math.random() * 0.1,
  }));

  let raf = 0;
  function frame() {
    ctx.clearRect(0, 0, w, h);
    const left = 96, right = w - 92;
    const laneY = (i) => 44 + i * ((h - 96) / 3);

    ctx.font = '11px ui-monospace, Menlo, monospace';
    ctx.textBaseline = 'middle';

    for (let i = 0; i < 4; i++) {
      const y = laneY(i);
      ctx.strokeStyle = 'rgba(14,58,40,.95)';
      ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(left, y); ctx.lineTo(right, y); ctx.stroke();
      ctx.fillStyle = '#6f8b7c';
      ctx.textAlign = 'right';
      ctx.fillText(NAMES[i], left - 12, y);
    }

    // The convergence point: every lane ends at one answer.
    ctx.strokeStyle = 'rgba(200,154,78,.5)';
    ctx.beginPath();
    for (let i = 0; i < 4; i++) { ctx.moveTo(right, laneY(i)); ctx.lineTo(right + 42, h / 2); }
    ctx.stroke();

    for (const d of dots) {
      d.t += d.v * 0.016;
      if (d.t > 1.25) d.t = -0.15;
      if (d.t < 0) continue;
      const p = Math.min(d.t, 1);
      const x = left + (right - left) * p;
      const y = laneY(d.lane);
      ctx.fillStyle = p >= 1 ? '#c89a4e' : '#4aa87a';
      ctx.beginPath(); ctx.arc(x, y, p >= 1 ? 2.6 : 2.1, 0, Math.PI * 2); ctx.fill();
    }

    ctx.fillStyle = '#e9f1ea';
    ctx.textAlign = 'left';
    ctx.font = '600 13px ui-serif, Palatino, Georgia, serif';
    ctx.fillText('one answer', right + 52, h / 2 - 8);
    ctx.fillStyle = '#6f8b7c';
    ctx.font = '11px ui-monospace, Menlo, monospace';
    ctx.fillText(total + ' programs', right + 52, h / 2 + 10);

    raf = requestAnimationFrame(frame);
  }

  if (REDUCED) { frame(); cancelAnimationFrame(raf); } else { frame(); }
}

// --------------------------------------------------------- measured bars

function bars() {
  const rows = document.querySelectorAll('.bar-row');
  if (!rows.length) return;
  const show = (row) => {
    const fill = row.querySelector('.bar-fill');
    if (fill) fill.style.width = (fill.dataset.pct || 0) + '%';
  };
  if (M.inView && !REDUCED) {
    rows.forEach((r) => M.inView(r, () => { show(r); }, { amount: 0.4 }));
  } else {
    rows.forEach(show);
  }
}

// ---------------------------------------------------------------- reveal

function reveal() {
  const items = document.querySelectorAll('.reveal');
  if (!items.length) return;
  if (REDUCED || !M.inView) { items.forEach((el) => el.classList.add('in')); return; }
  items.forEach((el) => M.inView(el, () => { el.classList.add('in'); }, { amount: 0.15 }));
}

stage();
capability();
engines();
bars();
reveal();
