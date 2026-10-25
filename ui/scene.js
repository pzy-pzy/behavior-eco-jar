/* 行为生态缸 —— 场景绘制。
   全部用 Canvas 2D：径向渐变 + 加色混合做柔光和辉光，
   不用 WebGL（透明 WebView2 下 WebGL 合成不稳）。
   静态的玻璃/水体/内阴影缓存成一张底图，只有参数变化时才重建；
   柔光用预渲染精灵，避免每帧创建大量渐变。 */
const Scene = (() => {
  const W = 224, H = 300;
  const CX = 112, CY = 104, R = 88, RW = R - 4;
  const TAU = Math.PI * 2;
  // 主体尺寸跟着培养皿走，方便整体缩放。
  const BODY_UNIT = 44 * (R / 132);

  /* 图纸(Archetype)= 体型 + 生态位运动 + 互动反应,每张图纸实现
     pose / drawBack / drawMain / drawFront 四个接口,环境(背景、
     微粒、气泡、焦散)由 Scene 统一负责。M0 仅浮伞型,绘制代码与
     旧版一致;新图纸(docs §5.4)按此接口增加。 */
  const DEFAULT_GENOME = {
    seed: 20261022,
    wob: [0.55, 0.32, 0.2],
    cilia_bias: 0,
    size: 1,
    hue_bias: 0,
    vacuoles: 3,
    satellites_bias: 0,
  };

  let canvas = null;
  let ctx = null;
  let dpr = 1;

  let bg = null;
  let bgCtx = null;
  let bgKey = '';

  let sprite = null;
  let spriteHue = -1;

  let motes = [];
  let bubbles = [];
  let reaction = 0;
  let renderScale = 1;
  let curArch = 'medusa';

  const mix = (a, b, t) => a + (b - a) * t;
  const rgba = (c, a) => `rgba(${c[0] | 0},${c[1] | 0},${c[2] | 0},${a < 0 ? 0 : a})`;

  function init(el) {
    canvas = el;
    ctx = el.getContext('2d');
    resize();
    window.addEventListener('resize', resize);
    seed();
  }

  function resize() {
    dpr = Math.min(3, (window.devicePixelRatio || 1) * renderScale);
    canvas.width = Math.round(W * dpr);
    canvas.height = Math.round(H * dpr);
    canvas.style.width = W + 'px';
    canvas.style.height = H + 'px';
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    bg = null;
    bgKey = '';
  }

  function setScale(scale) {
    const v = Math.max(0.5, Math.min(2, scale || 1));
    if (Math.abs(v - renderScale) < 0.001) return;
    renderScale = v;
    resize();
  }

  function seed() {
    let s = 20261022;
    const rnd = () => {
      s = (s * 1664525 + 1013904223) % 4294967296;
      return s / 4294967296;
    };
    motes = Array.from({ length: 28 }, () => ({
      x: CX + (rnd() - 0.5) * RW * 1.7,
      y: CY + (rnd() - 0.5) * RW * 1.7,
      z: 0.25 + rnd() * 0.75,
      p: rnd() * TAU,
      v: 0.1 + rnd() * 0.35,
    }));
  }

  function palette(hue) {
    return {
      body: [mix(126, 255, hue), mix(246, 214, hue), mix(224, 152, hue)],
      deep: [mix(10, 30, hue), mix(40, 38, hue), mix(44, 34, hue)],
      accent: [mix(96, 255, hue), mix(232, 186, hue), mix(208, 124, hue)],
    };
  }

  /* ---------- 预渲染精灵 ---------- */

  function ensureSprite(p) {
    if (sprite && Math.abs(spriteHue - p.body[0]) < 2) return;
    spriteHue = p.body[0];
    const s = document.createElement('canvas');
    s.width = s.height = 192;
    const g = s.getContext('2d');
    const rg = g.createRadialGradient(96, 96, 0, 96, 96, 96);
    rg.addColorStop(0, rgba(p.body, 1));
    rg.addColorStop(0.32, rgba(p.body, 0.42));
    rg.addColorStop(0.7, rgba(p.body, 0.1));
    rg.addColorStop(1, rgba(p.body, 0));
    g.fillStyle = rg;
    g.fillRect(0, 0, 192, 192);
    sprite = s;
  }

  function drawSprite(x, y, size, alpha) {
    if (!sprite || alpha <= 0.002) return;
    ctx.globalAlpha = Math.min(1, alpha);
    ctx.drawImage(sprite, x - size / 2, y - size / 2, size, size);
    ctx.globalAlpha = 1;
  }

  /* ---------- 静态底图 ---------- */

  function buildBackground(p, tr) {
    if (!bg) {
      bg = document.createElement('canvas');
      bg.width = canvas.width;
      bg.height = canvas.height;
      bgCtx = bg.getContext('2d');
      bgCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
    }
    const c = bgCtx;
    c.clearRect(0, 0, W, H);

    // 桌面投影
    c.save();
    c.translate(CX, CY + R + 14);
    c.scale(1, 0.2);
    const sh = c.createRadialGradient(0, 0, 4, 0, 0, 120);
    sh.addColorStop(0, 'rgba(0,0,0,0.26)');
    sh.addColorStop(1, 'rgba(0,0,0,0)');
    c.fillStyle = sh;
    c.beginPath();
    c.arc(0, 0, 120, 0, TAU);
    c.fill();
    c.restore();

    // 皿外微光
    const halo = c.createRadialGradient(CX, CY, R * 0.86, CX, CY, R * 1.42);
    halo.addColorStop(0, rgba(p.body, 0.06 + 0.1 * tr.glow));
    halo.addColorStop(1, rgba(p.body, 0));
    c.fillStyle = halo;
    c.beginPath();
    c.arc(CX, CY, R * 1.42, 0, TAU);
    c.fill();

    // 水体
    const g = c.createRadialGradient(CX - 42, CY - 58, 12, CX, CY, RW);
    g.addColorStop(0, rgba([p.deep[0] + 40, p.deep[1] + 54, p.deep[2] + 50], 0.62));
    g.addColorStop(0.55, rgba(p.deep, 0.72));
    g.addColorStop(1, rgba([p.deep[0] * 0.6, p.deep[1] * 0.76, p.deep[2] * 0.76], 0.86));
    c.beginPath();
    c.arc(CX, CY, RW, 0, TAU);
    c.fillStyle = g;
    c.fill();

    // 水面反光
    const top = c.createLinearGradient(0, CY - RW, 0, CY - RW * 0.05);
    top.addColorStop(0, 'rgba(214, 255, 250, 0.10)');
    top.addColorStop(1, 'rgba(214, 255, 250, 0)');
    c.fillStyle = top;
    c.beginPath();
    c.arc(CX, CY, RW, 0, TAU);
    c.fill();

    // 浑浊
    if (tr.turbidity > 0.02) {
      const h = c.createRadialGradient(CX, CY - 10, 10, CX, CY, RW);
      h.addColorStop(0, `rgba(158,150,104,${0.03 + tr.turbidity * 0.10})`);
      h.addColorStop(1, `rgba(110,104,72,${0.10 + tr.turbidity * 0.26})`);
      c.fillStyle = h;
      c.beginPath();
      c.arc(CX, CY, RW, 0, TAU);
      c.fill();
    }

    // 内阴影
    const inner = c.createRadialGradient(CX, CY, RW * 0.62, CX, CY, RW * 1.02);
    inner.addColorStop(0, 'rgba(0,0,0,0)');
    inner.addColorStop(0.72, 'rgba(0,0,0,0.08)');
    inner.addColorStop(1, 'rgba(0,0,0,0.34)');
    c.fillStyle = inner;
    c.beginPath();
    c.arc(CX, CY, RW, 0, TAU);
    c.fill();

    // 玻璃边缘
    c.beginPath();
    c.arc(CX, CY, R - 5, 0, TAU);
    c.lineWidth = 11;
    c.strokeStyle = 'rgba(168, 226, 214, 0.12)';
    c.stroke();

    c.beginPath();
    c.arc(CX, CY, R - 1, 0, TAU);
    c.lineWidth = 1.2;
    c.strokeStyle = 'rgba(226, 255, 248, 0.42)';
    c.stroke();

    c.beginPath();
    c.arc(CX, CY, R - 10, 0, TAU);
    c.lineWidth = 0.8;
    c.strokeStyle = 'rgba(190, 240, 232, 0.16)';
    c.stroke();

    c.lineCap = 'round';
    c.beginPath();
    c.arc(CX, CY, R - 6, -2.75, -2.02);
    c.lineWidth = 5;
    c.strokeStyle = `rgba(255,255,255,${0.10 + 0.10 * tr.glow})`;
    c.stroke();
    c.beginPath();
    c.arc(CX, CY, R - 7, -2.62, -2.18);
    c.lineWidth = 2;
    c.strokeStyle = 'rgba(255,255,255,0.26)';
    c.stroke();
  }

  /* ---------- 动态元素 ---------- */

  function drawCaustics(t, p, glow) {
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    for (let i = 0; i < 3; i++) {
      const x = CX + Math.sin(t * 0.13 + i * 1.9) * 46;
      const y = CY - 40 + Math.cos(t * 0.11 + i * 2.2) * 30;
      drawSprite(x, y, 190 + i * 24, 0.035 + 0.05 * glow);
    }
    ctx.restore();
  }

  function drawMotes(t, p, tr, m) {
    ctx.save();
    const boost = 0.75 + 0.09 * (m || 0);
    for (const mote of motes) {
      const x = mote.x + Math.sin(t * mote.v + mote.p) * 7;
      const y = mote.y + Math.cos(t * mote.v * 0.8 + mote.p) * 5;
      const a = (0.07 + mote.z * 0.18) * (1 - tr.turbidity * 0.35) * boost;
      ctx.fillStyle = rgba(p.body, a);
      ctx.beginPath();
      ctx.arc(x, y, 0.6 + mote.z * 1.3, 0, TAU);
      ctx.fill();
    }
    ctx.restore();
  }

  /// 繁盛阶段的共生小体：围绕主体缓慢公转，用细丝连着。
  function drawSatellites(t, c, p, tr, front) {
    const m = c.m || 0;
    const count = Math.max(
      0,
      Math.min(6, Math.floor((m - 2.3) * 3) + ((c.genome && c.genome.satellites_bias) || 0))
    );
    if (!count) return;
    for (let i = 0; i < count; i++) {
      const isFront = i % 2 === 0;
      if (isFront !== front) continue;
      const a = t * 0.28 + (i * TAU) / count;
      const rr = c.r * (1.75 + 0.18 * Math.sin(t * 0.6 + i));
      const x = c.x + Math.cos(a) * rr;
      const y = c.y + Math.sin(a) * rr * 0.85;
      const r = 2.2 + (i % 3) * 0.9;

      ctx.beginPath();
      ctx.moveTo(x, y);
      ctx.quadraticCurveTo(
        (x + c.x) / 2 + Math.sin(t + i) * 8,
        (y + c.y) / 2 + Math.cos(t + i) * 8,
        c.x,
        c.y
      );
      ctx.strokeStyle = rgba(p.body, 0.12);
      ctx.lineWidth = 0.7;
      ctx.stroke();

      ctx.beginPath();
      ctx.arc(x, y, r, 0, TAU);
      ctx.fillStyle = rgba(p.body, 0.55);
      ctx.fill();
      ctx.lineWidth = 0.6;
      ctx.strokeStyle = rgba([255, 255, 250], 0.35);
      ctx.stroke();
    }
  }

  function pose(t, st) {
    const tr = st.traits;
    const g = st.genome || DEFAULT_GENOME;
    // 成熟度 0..4:阶段 + 阶段内进度,形态连续过渡而不是跳变。
    const m = (st.stage || 0) + (st.growth || 0);
    const base =
      BODY_UNIT *
      (0.62 + 0.16 * m) *
      (g.size || 1) *
      (1 + 0.035 * Math.sin(t * 1.7) + reaction * 0.16);
    return {
      x: CX + Math.sin(t * 0.21) * 15 * tr.motility + Math.sin(t * 0.07) * 5,
      y: CY + Math.cos(t * 0.17) * 11 * tr.motility + Math.sin(t * 0.9) * 1.5,
      r: base,
      stage: st.stage,
      m,
      genome: g,
    };
  }

  function blob(x, y, r, t, wob, g) {
    const w = (g && g.wob) || [0.55, 0.32, 0.2];
    const N = 64;
    ctx.beginPath();
    for (let i = 0; i <= N; i++) {
      const a = (i / N) * TAU;
      const k =
        1 +
        wob *
          (w[0] * Math.sin(3 * a + t * 0.7) +
            w[1] * Math.sin(5 * a - t * 0.95) +
            w[2] * Math.sin(2 * a + t * 0.4));
      const px = x + Math.cos(a) * r * k;
      const py = y + Math.sin(a) * r * k;
      if (i === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
    }
    ctx.closePath();
  }

  function drawCilia(t, c, p, tr, front) {
    const m = c.m || 0;
    const n = Math.max(
      4,
      Math.min(16, Math.round(6 + 2.4 * m) + ((c.genome && c.genome.cilia_bias) || 0))
    );
    const grow = 0.32 + 0.11 * m;
    const fork = Math.max(0, Math.min(1, (m - 1.8) / 1.4));
    ctx.save();
    ctx.lineCap = 'round';
    for (let i = 0; i < n; i++) {
      const isFront = i % 2 === 0;
      if (isFront !== front) continue;
      const a = (i / n) * TAU + 0.35 * Math.sin(t * 0.25 + i);
      const len = c.r * grow * (0.8 + 0.45 * Math.sin(t * 0.8 + i * 1.3));
      const x0 = c.x + Math.cos(a) * c.r * 0.9;
      const y0 = c.y + Math.sin(a) * c.r * 0.9;
      const ca = a + 0.65 * Math.sin(t * 1.15 + i * 0.9);
      const mx = c.x + Math.cos(ca) * (c.r + len * 0.6);
      const my = c.y + Math.sin(ca) * (c.r + len * 0.6);
      const x1 = c.x + Math.cos(ca) * (c.r + len);
      const y1 = c.y + Math.sin(ca) * (c.r + len);
      ctx.beginPath();
      ctx.moveTo(x0, y0);
      ctx.quadraticCurveTo(mx, my, x1, y1);
      ctx.lineWidth = 1.7;
      ctx.strokeStyle = rgba(p.body, 0.26 + 0.3 * tr.glow);

      // 成体之后触须末端分叉
      if (fork > 0.05) {
        const tangent = ca + Math.PI / 2;
        const bl = len * 0.32 * fork;
        for (const side of [-1, 1]) {
          ctx.moveTo(x1, y1);
          ctx.lineTo(
            x1 + Math.cos(ca + tangent * side) * bl,
            y1 + Math.sin(ca + tangent * side) * bl
          );
        }
      }
      ctx.stroke();
    }
    ctx.restore();
  }

  function drawGlow(t, c, p, tr) {
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    const m = c.m || 0;
    drawSprite(c.x, c.y, c.r * (5.2 + 0.3 * m), (0.32 + 0.45 * tr.glow) * (0.82 + 0.09 * m));
    ctx.restore();
  }

  function drawBody(t, c, p, tr) {
    const m = c.m || 0;
    // 孢子阶段有一层半透明的壳，随成熟度淡出。
    const shell = Math.max(0, 1.25 - m);
    if (shell > 0.02) {
      ctx.beginPath();
      ctx.arc(c.x, c.y, c.r * 1.16, 0, TAU);
      ctx.lineWidth = 1.2;
      ctx.strokeStyle = rgba(p.accent, 0.3 * shell);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(c.x, c.y, c.r * 1.3, 0, TAU);
      ctx.lineWidth = 0.7;
      ctx.strokeStyle = rgba(p.accent, 0.14 * shell);
      ctx.stroke();
    }
    blob(c.x, c.y, c.r, t, 0.03 + 0.022 * Math.min(m, 3), c.genome);
    const bodyGrad = ctx.createRadialGradient(
      c.x - c.r * 0.35,
      c.y - c.r * 0.4,
      c.r * 0.08,
      c.x,
      c.y,
      c.r * 1.05
    );
    bodyGrad.addColorStop(0, rgba([p.body[0] + 40, p.body[1] + 12, p.body[2] + 14], 0.92));
    bodyGrad.addColorStop(0.55, rgba(p.body, 0.72));
    bodyGrad.addColorStop(1, rgba(p.accent, 0.38));
    ctx.fillStyle = bodyGrad;
    ctx.fill();

    ctx.lineWidth = 1.3;
    ctx.strokeStyle = rgba(p.body, 0.55 + 0.25 * tr.vitality);
    ctx.stroke();

    // 内侧薄膜
    blob(c.x, c.y, c.r * 0.82, t + 1.7, 0.05, c.genome);
    ctx.lineWidth = 0.9;
    ctx.strokeStyle = rgba([255, 255, 250], 0.1 + 0.1 * tr.vitality);
    ctx.stroke();

    // 液泡：成熟之后才出现
    const vcap = (c.genome && c.genome.vacuoles) || 3;
    const vac = Math.min(vcap, Math.max(0, Math.floor(m - 0.6)));
    for (let i = 0; i < vac; i++) {
      const a = 1.2 + i * 2.1 + Math.sin(t * 0.3 + i) * 0.2;
      const rr = c.r * (0.28 + 0.12 * i);
      const vx = c.x + Math.cos(a) * rr;
      const vy = c.y + Math.sin(a) * rr;
      ctx.beginPath();
      ctx.arc(vx, vy, c.r * (0.1 + 0.02 * (i % 2)), 0, TAU);
      ctx.fillStyle = rgba([255, 255, 250], 0.07);
      ctx.fill();
      ctx.strokeStyle = rgba(p.body, 0.3);
      ctx.lineWidth = 0.6;
      ctx.stroke();
    }

    // 细胞核
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    const nr = c.r * (0.3 + 0.03 * Math.sin(t * 1.9));
    const ng = ctx.createRadialGradient(c.x + c.r * 0.08, c.y - c.r * 0.1, 0, c.x, c.y, nr);
    ng.addColorStop(0, rgba([255, 255, 250], 0.55 + 0.3 * tr.glow));
    ng.addColorStop(0.6, rgba(p.body, 0.35));
    ng.addColorStop(1, rgba(p.body, 0));
    ctx.fillStyle = ng;
    ctx.beginPath();
    ctx.arc(c.x + c.r * 0.08, c.y - c.r * 0.1, nr, 0, TAU);
    ctx.fill();
    ctx.restore();

    // 第二细胞核：成体之后
    if (m > 2.2) {
      ctx.save();
      ctx.globalCompositeOperation = 'lighter';
      const sx = c.x - c.r * 0.34;
      const sy = c.y + c.r * 0.26;
      const sr = c.r * 0.16 * (1 + 0.05 * Math.sin(t * 2.3));
      const sg = ctx.createRadialGradient(sx, sy, 0, sx, sy, sr);
      sg.addColorStop(0, rgba([235, 255, 248], 0.35));
      sg.addColorStop(1, rgba(p.body, 0));
      ctx.fillStyle = sg;
      ctx.beginPath();
      ctx.arc(sx, sy, sr, 0, TAU);
      ctx.fill();
      ctx.restore();
    }

    // 发光点
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    for (let i = 0; i < 6; i++) {
      const a = t * 0.32 + (i * TAU) / 6;
      const rr = c.r * (0.46 + 0.12 * Math.sin(t * 0.7 + i));
      const x = c.x + Math.cos(a) * rr;
      const y = c.y + Math.sin(a) * rr * 0.92;
      const pulse = 0.45 + 0.55 * Math.abs(Math.sin(t * 1.1 + i * 0.9));
      drawSprite(x, y, 9 + pulse * 7, 0.5 * pulse * (0.4 + 0.6 * tr.glow));
    }
    ctx.restore();

    // 压力斑
    if (tr.spots > 0.04) {
      ctx.save();
      for (let i = 0; i < 5; i++) {
        const a = i * 1.7 + 0.4;
        const rr = c.r * (0.32 + (0.3 * ((i * 37) % 10)) / 10);
        const x = c.x + Math.cos(a) * rr;
        const y = c.y + Math.sin(a) * rr;
        ctx.fillStyle = `rgba(46,58,44,${tr.spots * 0.4})`;
        ctx.beginPath();
        ctx.ellipse(x, y, 2.2 + (i % 3), 1.6 + (i % 2), a, 0, TAU);
        ctx.fill();
      }
      ctx.restore();
    }
  }

  function updateBubbles(dt) {
    for (const b of bubbles) {
      b.life -= dt;
      b.y -= b.v * dt;
      b.x += Math.sin(b.y * 0.09 + b.p) * dt * 6;
    }
    bubbles = bubbles.filter((b) => b.life > 0 && b.y > CY - RW);
  }

  function drawBubbles() {
    if (!bubbles.length) return;
    ctx.save();
    for (const b of bubbles) {
      const a = Math.min(1, b.life * 2) * b.a;
      ctx.beginPath();
      ctx.arc(b.x, b.y, b.r, 0, TAU);
      ctx.strokeStyle = `rgba(220,255,248,${a})`;
      ctx.lineWidth = 0.9;
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(b.x - b.r * 0.3, b.y - b.r * 0.3, b.r * 0.3, 0, TAU);
      ctx.fillStyle = `rgba(255,255,255,${a * 0.7})`;
      ctx.fill();
    }
    ctx.restore();
  }

  /* ---------- 流线型(晨光种/夜洄种) ---------- */

  // 物种专属色调:晨光种金侧线、夜洄种蓝紫节段。
  const SPECIES_TINT = {
    dawn: [248, 214, 148],
    night: [148, 162, 240],
  };

  const fishState = { x: CX - 30, y: CY + 16, vx: 16, vy: 2, tx: CX, ty: CY, dart: 0 };

  function fishPose(t, st, dt) {
    const g = st.genome || DEFAULT_GENOME;
    const m = (st.stage || 0) + (st.growth || 0);
    const motility = (st.traits && st.traits.motility) || 0.5;
    dt = dt || dtDraw;
    fishState.dart = Math.max(0, fishState.dart - dt * 1.1);
    // 目标点缓慢游走,速度转向积分,朝向跟随速度——"游"而不是"漂"。
    fishState.tx = CX + Math.sin(t * 0.23) * 54 + Math.sin(t * 0.07) * 18;
    fishState.ty = CY + Math.cos(t * 0.19) * 36 + Math.sin(t * 0.53) * 10;
    const sp = (15 + 11 * motility) * (1 + 2.8 * fishState.dart);
    let ax = (fishState.tx - fishState.x) * 1.2 - fishState.vx * 0.85;
    let ay = (fishState.ty - fishState.y) * 1.2 - fishState.vy * 0.85;
    const al = Math.hypot(ax, ay) || 1;
    const am = Math.min(al, 46);
    fishState.vx += (ax / al) * am * dt;
    fishState.vy += (ay / al) * am * dt;
    const vl = Math.hypot(fishState.vx, fishState.vy) || 1;
    const vv = Math.min(vl, sp);
    fishState.vx = (fishState.vx / vl) * vv;
    fishState.vy = (fishState.vy / vl) * vv;
    fishState.x += fishState.vx * dt;
    fishState.y += fishState.vy * dt;
    const dx = fishState.x - CX;
    const dy = fishState.y - CY;
    const dd = Math.hypot(dx, dy);
    if (dd > RW - 18) {
      const k = (RW - 18) / dd;
      fishState.x = CX + dx * k;
      fishState.y = CY + dy * k;
      fishState.vx *= 0.5;
      fishState.vy *= 0.5;
    }
    return {
      x: fishState.x,
      y: fishState.y,
      r: BODY_UNIT * (0.6 + 0.13 * m) * (g.size || 1),
      m,
      heading: Math.atan2(fishState.vy, fishState.vx),
      genome: g,
      tint: SPECIES_TINT[st.species_id] || null,
      dart: fishState.dart,
    };
  }

  function drawFishBody(t, c, p, tr) {
    const L = c.r * 1.75;
    const Wd = c.r * 0.72;
    const col = c.tint || p.body;
    ctx.save();
    ctx.translate(c.x, c.y);
    ctx.rotate(c.heading);
    // 尾鳍(急冲时摆得更快)
    const tw = Math.sin(t * (8 + 6 * c.dart)) * Wd * 0.6;
    ctx.beginPath();
    ctx.moveTo(-L * 0.62, 0);
    ctx.quadraticCurveTo(-L * 0.95, -Wd * 0.55 + tw, -L * 1.22, -Wd * 0.8 + tw * 1.5);
    ctx.quadraticCurveTo(-L * 1.0, tw * 0.4, -L * 1.22, Wd * 0.8 + tw * 1.5);
    ctx.quadraticCurveTo(-L * 0.95, Wd * 0.55 + tw, -L * 0.62, 0);
    ctx.fillStyle = rgba(p.accent, 0.5);
    ctx.fill();
    // 背鳍
    ctx.beginPath();
    ctx.moveTo(-L * 0.12, -Wd * 0.9);
    ctx.quadraticCurveTo(L * 0.04, -Wd * 1.45, L * 0.3, -Wd * 0.75);
    ctx.closePath();
    ctx.fillStyle = rgba(col, 0.4);
    ctx.fill();
    // 身体
    const bgd = ctx.createRadialGradient(L * 0.3, -Wd * 0.2, Wd * 0.1, 0, 0, L * 0.8);
    bgd.addColorStop(0, rgba([col[0] + 36, col[1] + 20, col[2] + 20], 0.92));
    bgd.addColorStop(0.6, rgba(col, 0.68));
    bgd.addColorStop(1, rgba(p.accent, 0.4));
    ctx.beginPath();
    ctx.ellipse(0, 0, L * 0.72, Wd, 0, 0, TAU);
    ctx.fillStyle = bgd;
    ctx.fill();
    ctx.lineWidth = 1.1;
    ctx.strokeStyle = rgba(col, 0.5 + 0.25 * tr.vitality);
    ctx.stroke();
    // 侧线光带:晨光种金色 / 夜洄种蓝紫
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.beginPath();
    ctx.moveTo(-L * 0.58, 0);
    ctx.lineTo(L * 0.55, 0);
    ctx.lineCap = 'round';
    ctx.lineWidth = Wd * 0.3;
    ctx.strokeStyle = rgba(col, 0.22 + 0.5 * (tr.glow || 0));
    ctx.stroke();
    ctx.restore();
    // 眼
    ctx.beginPath();
    ctx.arc(L * 0.46, -Wd * 0.16, Wd * 0.15, 0, TAU);
    ctx.fillStyle = 'rgba(10,20,18,0.85)';
    ctx.fill();
    ctx.beginPath();
    ctx.arc(L * 0.49, -Wd * 0.2, Wd * 0.05, 0, TAU);
    ctx.fillStyle = 'rgba(255,255,250,0.8)';
    ctx.fill();
    ctx.restore();
  }

  /* ---------- 群游型(群游种) ---------- */

  const schoolState = { fishes: [], target: { x: CX, y: CY }, phase: 0, flash: 0, scatter: 0 };

  function ensureSchool(n) {
    while (schoolState.fishes.length < n) {
      const a = Math.random() * TAU;
      schoolState.fishes.push({
        x: CX + Math.cos(a) * 30 * Math.random(),
        y: CY + Math.sin(a) * 30 * Math.random(),
        vx: 20 * (Math.random() - 0.5),
        vy: 20 * (Math.random() - 0.5),
        s: 0.8 + Math.random() * 0.4,
      });
    }
  }

  function schoolPose(t, st, dt) {
    const g = st.genome || DEFAULT_GENOME;
    const m = (st.stage || 0) + (st.growth || 0);
    const motility = (st.traits && st.traits.motility) || 0.5;
    dt = dt || dtDraw;
    ensureSchool(6);
    schoolState.flash = Math.max(0, schoolState.flash - dt * 1.4);
    schoolState.scatter = Math.max(0, schoolState.scatter - dt * 0.8);
    schoolState.phase += dt;
    // 每隔几秒整群换向:换向瞬间集体闪光。
    if (schoolState.phase > 6) {
      schoolState.phase = 0;
      schoolState.flash = 1;
      const a = Math.random() * TAU;
      const rr = 20 + Math.random() * (RW - 40);
      schoolState.target = { x: CX + Math.cos(a) * rr, y: CY + Math.sin(a) * rr * 0.85 };
    }
    const fishes = schoolState.fishes;
    let cx0 = 0;
    let cy0 = 0;
    for (const f of fishes) {
      cx0 += f.x;
      cy0 += f.y;
    }
    cx0 /= fishes.length;
    cy0 /= fishes.length;
    const maxV = (26 + 16 * motility) * (0.7 + 0.1 * m);
    for (const f of fishes) {
      let ax = (schoolState.target.x - f.x) * 0.9;
      let ay = (schoolState.target.y - f.y) * 0.9;
      for (const o of fishes) {
        if (o === f) continue;
        const dx = f.x - o.x;
        const dy = f.y - o.y;
        const d2 = dx * dx + dy * dy;
        if (d2 < 200 && d2 > 0.01) {
          const d = Math.sqrt(d2);
          ax += (dx / d) * (14 - d) * 1.6;
          ay += (dy / d) * (14 - d) * 1.6;
        }
      }
      if (schoolState.scatter > 0.01) {
        const dx = f.x - cx0;
        const dy = f.y - cy0;
        const d = Math.hypot(dx, dy) || 1;
        ax += (dx / d) * 220 * schoolState.scatter;
        ay += (dy / d) * 220 * schoolState.scatter;
      }
      f.vx += ax * dt;
      f.vy += ay * dt;
      const vl = Math.hypot(f.vx, f.vy) || 1;
      const vv = Math.min(vl, maxV);
      f.vx = (f.vx / vl) * vv;
      f.vy = (f.vy / vl) * vv;
      f.x += f.vx * dt;
      f.y += f.vy * dt;
      const dx = f.x - CX;
      const dy = f.y - CY;
      const dd = Math.hypot(dx, dy);
      if (dd > RW - 12) {
        const k = (RW - 12) / dd;
        f.x = CX + dx * k;
        f.y = CY + dy * k;
        f.vx *= 0.7;
        f.vy *= 0.7;
      }
    }
    return {
      x: cx0,
      y: cy0,
      r: BODY_UNIT * 0.85 * (g.size || 1),
      m,
      fishes,
      flash: schoolState.flash,
      genome: g,
    };
  }

  function drawMiniFish(t, f, p, tr, extraA) {
    ctx.save();
    ctx.translate(f.x, f.y);
    ctx.rotate(Math.atan2(f.vy, f.vx));
    const L = 6.5 * f.s;
    const Wd = 2.6 * f.s;
    const tw = Math.sin(t * 11 + f.s * 9) * Wd * 0.8;
    ctx.beginPath();
    ctx.moveTo(-L, 0);
    ctx.lineTo(-L * 1.7, -Wd * 1.1 + tw);
    ctx.lineTo(-L * 1.5, tw * 0.5);
    ctx.lineTo(-L * 1.7, Wd * 1.1 + tw);
    ctx.closePath();
    ctx.fillStyle = rgba(p.accent, 0.45 * extraA);
    ctx.fill();
    ctx.beginPath();
    ctx.ellipse(0, 0, L, Wd, 0, 0, TAU);
    ctx.fillStyle = rgba(p.body, 0.8 * extraA);
    ctx.fill();
    ctx.restore();
  }

  /* ---------- 辐射型(沉玉种) ---------- */

  const starState = { x: CX - 36, dir: 1, curl: [0, 0, 0, 0, 0], pause: 0 };

  function starPose(t, st, dt) {
    const g = st.genome || DEFAULT_GENOME;
    const m = (st.stage || 0) + (st.growth || 0);
    const motility = (st.traits && st.traits.motility) || 0.5;
    dt = dt || dtDraw;
    for (let i = 0; i < 5; i++) starState.curl[i] *= Math.max(0, 1 - dt * 1.6);
    starState.pause -= dt;
    if (starState.pause <= 0) {
      starState.x += starState.dir * (2.4 + 3.2 * motility) * dt;
      if (starState.x > CX + RW * 0.58) starState.dir = -1;
      if (starState.x < CX - RW * 0.58) starState.dir = 1;
      if (Math.sin(t * 0.17) > 0.989) starState.pause = 2 + Math.random() * 3;
    }
    return {
      x: starState.x,
      y: CY + RW * 0.72 + Math.sin(t * 0.7) * 1.4,
      r: BODY_UNIT * (0.5 + 0.12 * m) * (g.size || 1),
      m,
      heading: starState.dir > 0 ? 0 : Math.PI,
      curl: starState.curl,
      genome: g,
    };
  }

  function drawStarBody(t, c, p, tr) {
    const n = 5;
    const base = c.heading;
    ctx.save();
    ctx.translate(c.x, c.y);
    // 五条腕:中间那条朝前进方向
    for (let i = 0; i < n; i++) {
      const a = base + (i - 2) * (TAU / n) + Math.sin(t * 0.5 + i * 1.3) * 0.07;
      const curl = c.curl[i] || 0;
      const len = c.r * 1.42 * (0.92 + 0.08 * Math.sin(t * 0.8 + i)) * (1 - 0.42 * curl);
      const px = Math.cos(a + Math.PI / 2) * c.r * 0.3;
      const py = Math.sin(a + Math.PI / 2) * c.r * 0.3;
      const tipX = Math.cos(a + curl * 0.9) * len;
      const tipY = Math.sin(a + curl * 0.9) * len;
      ctx.beginPath();
      ctx.moveTo(Math.cos(a) * c.r * 0.3 + px, Math.sin(a) * c.r * 0.3 + py);
      ctx.quadraticCurveTo(
        Math.cos(a) * len * 0.55 + px * 0.4,
        Math.sin(a) * len * 0.55 + py * 0.4,
        tipX,
        tipY
      );
      ctx.quadraticCurveTo(
        Math.cos(a) * len * 0.55 - px * 0.4,
        Math.sin(a) * len * 0.55 - py * 0.4,
        Math.cos(a) * c.r * 0.3 - px,
        Math.sin(a) * c.r * 0.3 - py
      );
      ctx.closePath();
      ctx.fillStyle = rgba(p.body, 0.6 + 0.15 * tr.vitality);
      ctx.fill();
      ctx.lineWidth = 0.8;
      ctx.strokeStyle = rgba(p.body, 0.55);
      ctx.stroke();
    }
    // 中心体盘
    blob(0, 0, c.r * 0.55, t, 0.05, c.genome);
    ctx.fillStyle = rgba(p.body, 0.78);
    ctx.fill();
    ctx.strokeStyle = rgba(p.accent, 0.4);
    ctx.lineWidth = 0.9;
    ctx.stroke();
    // 腕尖五点微光(沉玉种的光语言)
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    for (let i = 0; i < n; i++) {
      const a = base + (i - 2) * (TAU / n) + Math.sin(t * 0.5 + i * 1.3) * 0.07;
      const curl = c.curl[i] || 0;
      const len = c.r * 1.42 * (1 - 0.42 * curl);
      const pulse = 0.5 + 0.5 * Math.sin(t * 1.4 + i * 1.9);
      drawSprite(
        Math.cos(a + curl * 0.9) * len,
        Math.sin(a + curl * 0.9) * len,
        8 + pulse * 6,
        (0.3 + 0.5 * (tr.glow || 0)) * pulse
      );
    }
    ctx.restore();
    // 压力斑
    if (tr.spots > 0.04) {
      for (let i = 0; i < 4; i++) {
        const a = i * 1.9 + 0.7;
        ctx.fillStyle = `rgba(46,58,44,${tr.spots * 0.4})`;
        ctx.beginPath();
        ctx.ellipse(Math.cos(a) * c.r * 0.4, Math.sin(a) * c.r * 0.4, 2, 1.5, a, 0, TAU);
        ctx.fill();
      }
    }
    ctx.restore();
  }

  /* ---------- 图纸注册表 ---------- */

  const Archetypes = {
    // 浮伞型:中层浮游,收缩-滑行脉冲推进(即旧版主体,绘制代码未动)。
    medusa: {
      pose,
      drawBack(t, c, p, tr) {
        drawSatellites(t, c, p, tr, false);
        drawCilia(t, c, p, tr, false);
      },
      drawMain(t, c, p, tr) {
        drawGlow(t, c, p, tr);
        drawBody(t, c, p, tr);
      },
      drawFront(t, c, p, tr) {
        drawCilia(t, c, p, tr, true);
        drawSatellites(t, c, p, tr, true);
      },
    },
    // 流线型:全域巡游,朝向跟随速度;点击急冲(晨光种/夜洄种)。
    fish: {
      pose: fishPose,
      react() {
        fishState.dart = 1;
      },
      drawBack(t, c, p, tr) {
        ctx.save();
        ctx.globalCompositeOperation = 'lighter';
        drawSprite(c.x, c.y, c.r * 4.6, 0.16 + 0.4 * (tr.glow || 0));
        ctx.restore();
      },
      drawMain: drawFishBody,
      drawFront() {},
    },
    // 群游型:6 条小鱼简化群集,整群周期性换向闪光;点击惊散(群游种)。
    school: {
      pose: schoolPose,
      react() {
        schoolState.scatter = 1;
      },
      drawBack(t, c, p, tr) {
        ctx.save();
        ctx.globalCompositeOperation = 'lighter';
        drawSprite(c.x, c.y, c.r * 6, (0.14 + 0.3 * (tr.glow || 0)) * (1 + 0.7 * c.flash));
        ctx.restore();
      },
      drawMain(t, c, p, tr) {
        const a = 0.85 + 0.15 * c.flash;
        for (const f of c.fishes) drawMiniFish(t, f, p, tr, a);
        if (c.flash > 0.25) {
          ctx.save();
          ctx.globalCompositeOperation = 'lighter';
          for (const f of c.fishes) drawSprite(f.x, f.y, 10, 0.3 * c.flash);
          ctx.restore();
        }
      },
      drawFront() {},
    },
    // 辐射型:底栖极慢爬行,腕尖五点微光;点击单腕卷曲(沉玉种)。
    star: {
      pose: starPose,
      react() {
        starState.curl[(Math.random() * 5) | 0] = 1;
      },
      drawBack(t, c, p, tr) {
        ctx.save();
        ctx.globalCompositeOperation = 'lighter';
        drawSprite(c.x, c.y, c.r * 3.4, 0.1 + 0.3 * (tr.glow || 0));
        ctx.restore();
      },
      drawMain: drawStarBody,
      drawFront() {},
    },
  };

  /* ---------- 主循环 ---------- */

  function frame(ts, st, dt) {
    if (!ctx || !st) return;
    const t = ts / 1000;
    const tr = st.traits || {};
    const spec = {
      ...st,
      genome: st.genome || DEFAULT_GENOME,
      archetype: st.archetype || 'medusa',
    };
    const hue = (tr.hue || 0) + (spec.genome.hue_bias || 0);
    const p = palette(hue);
    reaction = Math.max(0, reaction - dt * 2.2);

    ensureSprite(p);

    const key = `${hue.toFixed(2)}|${(tr.turbidity || 0).toFixed(2)}|${(tr.glow || 0).toFixed(2)}|${dpr}`;
    if (key !== bgKey) {
      bgKey = key;
      buildBackground(p, tr);
    }

    ctx.clearRect(0, 0, W, H);
    if (bg) ctx.drawImage(bg, 0, 0, W, H);

    ctx.save();
    ctx.beginPath();
    ctx.arc(CX, CY, RW, 0, TAU);
    ctx.clip();

    drawCaustics(t, p, tr.glow || 0);

    const arch = Archetypes[spec.archetype] || Archetypes.medusa;
    curArch = spec.archetype in Archetypes ? spec.archetype : 'medusa';
    const c = arch.pose(t, spec, dt || dtDraw);
    drawMotes(t, p, tr, c.m);

    arch.drawBack(t, c, p, tr);
    arch.drawMain(t, c, p, tr);
    arch.drawFront(t, c, p, tr);

    updateBubbles(dt);
    drawBubbles();
    ctx.restore();
  }

  function burst(n = 12) {
    for (let i = 0; i < n; i++) {
      const a = Math.random() * TAU;
      const rr = Math.random() * RW * 0.8;
      bubbles.push({
        x: CX + Math.cos(a) * rr,
        y: CY + Math.sin(a) * rr,
        r: 1.2 + Math.random() * 2.6,
        v: 14 + Math.random() * 22,
        life: 1.6 + Math.random() * 2.4,
        a: 0.25 + Math.random() * 0.3,
        p: Math.random() * TAU,
      });
    }
    if (bubbles.length > 90) bubbles = bubbles.slice(-90);
  }

  function react() {
    reaction = 1;
    const a = Archetypes[curArch];
    if (a && a.react) a.react();
  }

  return { init, frame, burst, react, setScale };
})();
