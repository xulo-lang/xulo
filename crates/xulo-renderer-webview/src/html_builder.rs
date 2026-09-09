//! Pure JS Canvas2D HTML builder — no WASM dependency.
//!
//! [`build_html_with_context`] generates an HTML page that renders a widget
//! tree using the browser's native Canvas 2D API. The layout engine runs
//! entirely in JavaScript, eliminating the WASM compilation step.

use crate::webview_backend::WebviewSize;

/// Build a full HTML page that renders `tree_json` using a pure JS Canvas2D
/// renderer. No WASM is embedded — the layout + paint logic runs entirely
/// in JavaScript.
pub fn build_html_with_context(
    tree_json: &str,
    size: WebviewSize,
    background: (u8, u8, u8),
) -> String {
    let (width, height) = size.pixels();
    let bg = format!("rgb({},{},{})", background.0, background.1, background.2);

    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>html,body{{margin:0;padding:0;background:{bg};overflow:hidden;}}</style>
</head>
<body>
<div id="wrap" style="position:relative;display:inline-block;">
<canvas id="screen"></canvas>
<div id="inputs"></div>
</div>
<script>
const TREE = {tree_json};
let W = {width};
let H = {height};
const CELL_W = 8, CELL_H = 16;
const canvas = document.getElementById('screen');
const dpr = window.devicePixelRatio || 1;
canvas.width = W * dpr;
canvas.height = H * dpr;
canvas.style.width = W + 'px';
canvas.style.height = H + 'px';
const ctx = canvas.getContext('2d');
ctx.scale(dpr, dpr);
ctx.textBaseline = 'top';
ctx.fillStyle = '{bg}';
ctx.fillRect(0, 0, W, H);

const buttonRects = [];
const inputRects = [];

function uw(w) {{
  if (!w) return null;
  if (typeof w === 'string') return {{ variant: '_str', data: w }};
  const keys = Object.keys(w);
  if (keys.length === 1) {{
    const v = keys[0];
    // Valid widget variants
    if (['Screen','VStack','HStack','Fill','Text','Button','Input','ScrollView','Unknown'].includes(v)) {{
      return {{ variant: v, data: w[v] }};
    }}
  }}
  // Fallback: treat as unknown
  return {{ variant: '_unknown', data: w }};
}}

function estHeight(w, aw) {{
  if (!w) return 20;
  const u = uw(w);
  if (!u) return 20;
  switch (u.variant) {{
    case '_str': return 16;
    case 'Text': {{
      const s = u.data.style || {{}};
      if (s.height === 'full') return -1;
      if (typeof s.height === 'number') return s.height;
      const fs = s.font_size || 12;
      const pad = s.padding || 0;
      // Estimate line count based on available width
      const maxW = (typeof aw === 'number' && aw > 0) ? aw - pad * 2 : 9999;
      const charW = fs * 0.6;
      const textLen = (u.data.text || '').length * charW;
      const lines = Math.max(1, Math.ceil(textLen / Math.max(1, maxW)));
      return lines * fs * 1.2 + pad * 2;
    }}
    case 'Button': return 28;
    case 'Input': return 24;
    case 'VStack': {{
      const s = u.data.style || {{}};
      if (s.height === 'full') return -1; // signal: expand
      if (typeof s.height === 'number') return s.height;
      let h = 0;
      const ch = u.data.children || [];
      for (let i = 0; i < ch.length; i++) {{
        const eh = estHeight(ch[i], aw);
        if (eh < 0) return -1; // has expand child → expand
        h += eh;
      }}
      h += (u.data.spacing || 0) * Math.max(0, ch.length - 1);
      return h + (s.padding || 0) * 2;
    }}
    case 'HStack': {{
      const s = u.data.style || {{}};
      if (s.height === 'full') return -1;
      if (typeof s.height === 'number') return s.height;
      let mh = 0;
      for (const c of (u.data.children || [])) {{
        const eh = estHeight(c, aw);
        if (eh < 0) return -1;
        mh = Math.max(mh, eh);
      }}
      return mh + (s.padding || 0) * 2;
    }}
    case 'Fill': {{
      return -1; // Fill always expands
    }}
    default: return 20;
  }}
}}

function estWidth(w, ah) {{
  if (!w) return 100;
  const u = uw(w);
  if (!u) return 100;
  switch (u.variant) {{
    case '_str': return (u.data || '').length * 7.2;
    case 'Text': {{
      const s = u.data.style || {{}};
      if (s.width === 'full') return -1;
      if (typeof s.width === 'number') return s.width;
      return (u.data.text || '').length * (s.font_size || 12) * 0.6 + (s.padding || 0) * 2;
    }}
    case 'Button': return (u.data.label || '').length * 8 + 16;
    case 'Input': return u.data.width || 200;
    // Containers default to 100% width (matching taffy behavior)
    case 'VStack': {{
      const s = u.data.style || {{}};
      if (typeof s.width === 'number') return s.width;
      if (s.width === 'full') return -1;
      return -1; // containers default to 100%
    }}
    case 'HStack': {{
      const s = u.data.style || {{}};
      if (typeof s.width === 'number') return s.width;
      if (s.width === 'full') return -1;
      return -1; // containers default to 100%
    }}
    case 'Fill': return -1;
    case 'ScrollView': return -1;
    default: return 100;
  }}
}}

function col(c) {{
  if (!c) return 'rgb(0,0,0)';
  return 'rgb(' + c.r + ',' + c.g + ',' + c.b + ')';
}}

function render(w, x, y, aw, ah) {{
  if (!w) return;
  const u = uw(w);
  if (!u) return;

  switch (u.variant) {{
    case '_str': {{
      ctx.fillStyle = 'rgb(255,255,255)';
      ctx.font = '12px monospace';
      ctx.fillText(u.data, x, y);
      break;
    }}
    case '_unknown': {{
      ctx.fillStyle = '#404040';
      ctx.fillRect(x, y, aw, ah);
      ctx.fillStyle = '#808080';
      ctx.font = '10px monospace';
      ctx.fillText(JSON.stringify(u.data).slice(0, 30), x + 4, y + 4);
      break;
    }}
    case 'Screen': {{
      const d = u.data;
      if (d.background) {{
        ctx.fillStyle = col(d.background);
        ctx.fillRect(x, y, aw, ah);
      }}
      for (const c of (d.children || [])) render(c, x, y, aw, ah);
      break;
    }}
    case 'VStack': {{
      const d = u.data;
      const s = d.style || {{}};
      if (s.background_color) {{
        ctx.fillStyle = col(s.background_color);
        ctx.fillRect(x, y, aw, ah);
      }}
      const pad = s.padding || 0;
      const ix = x + pad, iy = y + pad, iw = aw - pad * 2;
      const ih = ah - pad * 2;
      const ch = d.children || [];
      const sp = d.spacing || 0;

      // Two-pass layout: measure non-expanding, then distribute remaining space
      let totalFixed = 0;
      let expandCount = 0;
      const sizes = [];
      for (const c of ch) {{
        const eh = estHeight(c, iw);
        if (eh < 0) {{ expandCount++; sizes.push(-1); }}
        else {{ totalFixed += eh; sizes.push(eh); }}
      }}
      const remaining = ih - totalFixed - sp * Math.max(0, ch.length - 1);
      const expandH = expandCount > 0 ? Math.max(0, remaining / expandCount) : 0;

      // Resolve sizes
      const resolved = sizes.map((s, i) => s < 0 ? expandH : s);

      // justifyContent
      const justify = s.justify || 'start';
      if (justify === 'space-between' && ch.length > 1) {{
        const totalContent = resolved.reduce((a, b) => a + b, 0);
        const gap = Math.max(0, ih - totalContent) / (ch.length - 1);
        let cy = iy;
        for (let i = 0; i < ch.length; i++) {{
          render(ch[i], ix, cy, iw, resolved[i]);
          cy += resolved[i] + gap;
        }}
        break;
      }}
      let startY = iy;
      if (justify === 'center') {{
        startY = iy + Math.max(0, remaining / 2);
      }} else if (justify === 'end') {{
        startY = iy + Math.max(0, remaining);
      }}
      let cy = startY;
      for (let i = 0; i < ch.length; i++) {{
        render(ch[i], ix, cy, iw, resolved[i]);
        cy += resolved[i] + sp;
      }}
      break;
    }}
    case 'HStack': {{
      const d = u.data;
      const s = d.style || {{}};
      if (s.background_color) {{
        ctx.fillStyle = col(s.background_color);
        ctx.fillRect(x, y, aw, ah);
      }}
      const pad = s.padding || 0;
      const ix = x + pad, iy = y + pad, ih = ah - pad * 2;
      const iw = aw - pad * 2;
      const ch = d.children || [];
      const sp = d.spacing || 0;

      // Measure intrinsic widths; distribute space to all children by default
      let totalFixed = 0;
      let expandCount = 0;
      const sizes = [];
      const canExpand = [];
      for (const c of ch) {{
        const ew = estWidth(c, ih);
        const u2 = uw(c);
        const s2 = u2?.data?.style || {{}};
        // Children with explicit numeric width are fixed; everything else expands
        if (ew < 0 || typeof s2.width !== 'number') {{
          expandCount++;
          sizes.push(-1);
          canExpand.push(true);
        }} else {{
          totalFixed += ew;
          sizes.push(ew);
          canExpand.push(false);
        }}
      }}

      const justify = s.justify || 'start';
      const totalSpacing = sp * Math.max(0, ch.length - 1);
      const remaining = iw - totalFixed - totalSpacing;

      // Resolve widths
      let resolved;
      if (expandCount > 0 && remaining > 0) {{
        const expandW = remaining / expandCount;
        resolved = sizes.map((sz, i) => canExpand[i] ? Math.max(sz, expandW) : sz);
      }} else {{
        resolved = sizes.slice();
      }}

      if (justify === 'space-between' && ch.length > 1) {{
        const totalContent = resolved.reduce((a, b) => a + b, 0);
        const gap = Math.max(0, iw - totalContent) / (ch.length - 1);
        let cx = ix;
        for (let i = 0; i < ch.length; i++) {{
          render(ch[i], cx, iy, resolved[i], ih);
          cx += resolved[i] + gap;
        }}
        break;
      }}
      let startX = ix;
      if (justify === 'center') {{
        startX = ix + Math.max(0, remaining / 2);
      }} else if (justify === 'end') {{
        startX = ix + Math.max(0, remaining);
      }}
      let cx = startX;
      for (let i = 0; i < ch.length; i++) {{
        render(ch[i], cx, iy, resolved[i], ih);
        cx += resolved[i] + sp;
      }}
      break;
    }}
    case 'Fill': {{
      const d = u.data;
      const s = d.style || {{}};
      if (s.background_color) {{
        ctx.fillStyle = col(s.background_color);
        ctx.fillRect(x, y, aw, ah);
      }}
      const pad = s.padding || 0;
      const ix = x + pad, iy = y + pad, iw = aw - pad * 2, ih = ah - pad * 2;
      for (const c of (d.children || [])) render(c, ix, iy, iw, ih);
      break;
    }}
    case 'Text': {{
      const d = u.data;
      const s = d.style || {{}};
      const pad = s.padding || 0;
      const fs = s.font_size || 12;
      const fw = s.font_weight === 'bold' ? 'bold ' : '';
      ctx.font = fw + fs + 'px monospace';
      // Box model: draw background rectangle filling allocated space
      if (s.background_color) {{
        ctx.fillStyle = col(s.background_color);
        ctx.fillRect(x, y, aw, ah);
      }}
      // Clip to box
      ctx.save();
      ctx.beginPath();
      ctx.rect(x, y, aw, ah);
      ctx.clip();
      ctx.fillStyle = col(d.color || s.color || {{r:255,g:255,b:255}});
      // Word wrap: split text into lines that fit within (aw - pad*2)
      const maxW = aw - pad * 2;
      const rawText = d.text || '';
      const lines = [];
      let cur = '';
      for (const ch of rawText) {{
        const test = cur + ch;
        if (ctx.measureText(test).width > maxW && cur.length > 0) {{
          lines.push(cur);
          cur = ch;
        }} else {{
          cur = test;
        }}
      }}
      if (cur) lines.push(cur);
      // Render lines, vertically centered
      const lineH = fs * 1.2;
      const totalH = lines.length * lineH;
      let ty = y + pad + Math.max(0, (ah - pad * 2 - totalH) / 2) + fs * 0.8;
      for (const line of lines) {{
        ctx.fillText(line, x + pad, ty);
        ty += lineH;
      }}
      ctx.restore();
      break;
    }}
    case 'Button': {{
      const d = u.data;
      const s = d.style || {{}};
      const bg = s.background_color || {{r:74,g:144,b:217}};
      ctx.fillStyle = col(bg);
      ctx.beginPath();
      ctx.roundRect(x, y, aw, ah, 4);
      ctx.fill();
      if (s.border_color) {{
        ctx.strokeStyle = col(s.border_color);
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.roundRect(x, y, aw, ah, 4);
        ctx.stroke();
      }}
      const fg = s.color || {{r:255,g:255,b:255}};
      ctx.fillStyle = col(fg);
      ctx.font = '12px monospace';
      ctx.fillText(d.label || '', x + 8, y + 4);
      buttonRects.push({{x, y, w: aw, h: ah}});
      break;
    }}
    case 'Input': {{
      const d = u.data;
      const s = d.style || {{}};
      const bg = s.background_color || {{r:255,g:255,b:255}};
      ctx.fillStyle = col(bg);
      ctx.fillRect(x, y, aw, ah);
      const bc = s.border_color || {{r:128,g:128,b:128}};
      ctx.strokeStyle = col(bc);
      ctx.lineWidth = 1;
      ctx.strokeRect(x, y, aw, ah);
      const display = d.value || d.placeholder || '';
      const tc = d.value ? (s.color || {{r:0,g:0,b:0}}) : {{r:128,g:128,b:128}};
      ctx.fillStyle = col(tc);
      ctx.font = '12px monospace';
      ctx.fillText(display, x + 4, y + 4);
      inputRects.push({{x, y, w: aw, h: ah}});
      break;
    }}
    case 'ScrollView': {{
      const d = u.data;
      const s = d.style || {{}};
      const pad = s.padding || 0;
      for (const c of (d.children || [])) render(c, x + pad, y + pad, aw - pad * 2, ah - pad * 2);
      break;
    }}
    default: {{
      ctx.fillStyle = '#404040';
      ctx.fillRect(x, y, aw, ah);
      ctx.fillStyle = '#808080';
      ctx.font = '10px monospace';
      ctx.fillText(u.variant, x + 4, y + 4);
    }}
  }}
}}

function redraw(tree) {{
  buttonRects.length = 0;
  inputRects.length = 0;
  ctx.clearRect(0, 0, W, H);
  render(tree, 0, 0, W, H);
  syncInputs();
}}

// ── Input field management ──
const inputContainer = document.getElementById('inputs');
inputContainer.style.position = 'absolute';
inputContainer.style.top = '0';
inputContainer.style.left = '0';
inputContainer.style.pointerEvents = 'none';
const inputEls = [];

function findInputs(w) {{
  if (!w) return;
  const u = uw(w);
  if (!u) return;
  if (u.variant === 'Input') inputEls._nodes = inputEls._nodes || [];
  if (u.variant === 'Input' && inputEls._nodes) inputEls._nodes.push(u.data);
  for (const c of (u.data.children || [])) findInputs(c);
}}

function syncInputs() {{
  const nodes = [];
  findInputNodes(TREE, nodes);
  while (inputEls.length < inputRects.length) {{
    const el = document.createElement('input');
    el.type = 'text';
    el.style.position = 'absolute';
    el.style.font = '12px monospace';
    el.style.border = 'none';
    el.style.outline = 'none';
    el.style.background = 'transparent';
    el.style.padding = '0';
    el.style.margin = '0';
    el.style.pointerEvents = 'auto';
    el.style.boxSizing = 'border-box';
    const idx = inputEls.length;
    el.addEventListener('input', (e) => {{
      window.ipc.postMessage('input ' + idx + ' ' + e.target.value);
    }});
    inputContainer.appendChild(el);
    inputEls.push(el);
  }}
  while (inputEls.length > inputRects.length) {{
    const el = inputEls.pop();
    inputContainer.removeChild(el);
  }}
  for (let i = 0; i < inputEls.length; i++) {{
    const r = inputRects[i];
    const el = inputEls[i];
    el.style.left = r.x + 'px';
    el.style.top = r.y + 'px';
    el.style.width = r.w + 'px';
    el.style.height = r.h + 'px';
    const node = nodes[i];
    if (node && document.activeElement !== el) {{
      el.value = node.value || '';
      el.placeholder = node.placeholder || '';
    }}
  }}
}}

function findInputNodes(w, out) {{
  if (!w) return;
  const u = uw(w);
  if (!u) return;
  if (u.variant === 'Input') out.push(u.data);
  for (const c of (u.data.children || [])) findInputNodes(c, out);
}}

redraw(TREE);
window.redraw = redraw;

// ── Click handling ──
canvas.addEventListener('click', (e) => {{
  const r = canvas.getBoundingClientRect();
  const mx = e.clientX - r.left;
  const my = e.clientY - r.top;
  for (let i = buttonRects.length - 1; i >= 0; i--) {{
    const b = buttonRects[i];
    if (mx >= b.x && mx < b.x + b.w && my >= b.y && my < b.y + b.h) {{
      window.ipc.postMessage('click ' + i);
      return;
    }}
  }}
  for (let i = inputRects.length - 1; i >= 0; i--) {{
    const ir = inputRects[i];
    if (mx >= ir.x && mx < ir.x + ir.w && my >= ir.y && my < ir.y + ir.h && inputEls[i]) {{
      inputEls[i].focus();
      return;
    }}
  }}
}});

// ── Resize ──
let __resizeTimer = null;
let __resizeRaf = 0;
function resize(w, h) {{
  clearTimeout(__resizeTimer);
  __resizeTimer = setTimeout(() => {{
    w = Math.max(1, w);
    h = Math.max(1, h);
    const cols = Math.floor(w / CELL_W) || 1;
    const rows = Math.floor(h / CELL_H) || 1;
    const snapW = cols * CELL_W;
    const snapH = rows * CELL_H;
    if (snapW === W && snapH === H) return;
    if (__resizeRaf) cancelAnimationFrame(__resizeRaf);
    __resizeRaf = requestAnimationFrame(() => {{
      W = snapW;
      H = snapH;
      canvas.width = W * dpr;
      canvas.height = H * dpr;
      canvas.style.width = W + 'px';
      canvas.style.height = H + 'px';
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.textBaseline = 'top';
      redraw(TREE);
      __resizeRaf = 0;
    }});
  }}, 150);
}}
window.resize = resize;
new ResizeObserver(() => {{
  const rect = document.body.getBoundingClientRect();
  resize(rect.width, rect.height);
}}).observe(document.body);
</script>
</body>
</html>"#
    )
}
