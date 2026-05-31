interface ExcalidrawElement {
  type: string;
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  strokeColor?: string;
  backgroundColor?: string;
  fillStyle?: string;
  strokeWidth?: number;
  roughness?: number;
  opacity?: number;
  text?: string;
  fontSize?: number;
  fontFamily?: number;
  textAlign?: string;
  verticalAlign?: string;
  points?: number[][];
  roundness?: { type: number };
  containerId?: string;
  endArrowhead?: string | null;
  strokeStyle?: string;
}

interface ExcalidrawFile {
  type?: string;
  version?: number;
  elements?: ExcalidrawElement[];
  appState?: { viewBackgroundColor?: string };
}

function roughSeed(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i++) {
    hash = ((hash << 5) - hash) + id.charCodeAt(i);
    hash |= 0;
  }
  return Math.abs(hash);
}

function randomOffset(seed: number, idx: number, amount: number): number {
  const x = Math.sin(seed * 9301 + idx * 49297 + 233280) * 49297;
  return ((x - Math.floor(x)) - 0.5) * amount;
}

function roughLine(x1: number, y1: number, x2: number, y2: number, seed: number, roughness: number = 1): string {
  if (roughness === 0) return `M${x1},${y1} L${x2},${y2}`;
  const r = roughness * 1.5;
  return `M${x1 + randomOffset(seed, 1, r)},${y1 + randomOffset(seed, 2, r)} L${x2 + randomOffset(seed, 3, r)},${y2 + randomOffset(seed, 4, r)}`;
}

function roughRect(x: number, y: number, w: number, h: number, seed: number, roughness: number = 1): string {
  const r = roughness * 1.5;
  const sx = x + randomOffset(seed, 1, r);
  const sy = y + randomOffset(seed, 2, r);
  const ex = x + w + randomOffset(seed, 3, r);
  const ey = y + h + randomOffset(seed, 4, r);
  return `${roughLine(sx, sy, ex, sy, seed * 2, roughness)} ${roughLine(ex, sy, ex, ey, seed * 3, roughness)} ${roughLine(ex, ey, sx, ey, seed * 4, roughness)} ${roughLine(sx, ey, sx, sy, seed * 5, roughness)}`;
}

function roughEllipse(cx: number, cy: number, w: number, h: number, seed: number, roughness: number = 1): string {
  const rx = w / 2;
  const ry = h / 2;
  if (roughness === 0) {
    return `M${cx - rx},${cy} A${rx},${ry} 0 1,1 ${cx + rx},${cy} A${rx},${ry} 0 1,1 ${cx - rx},${cy}`;
  }
  const r = roughness * 2;
  const steps = 24;
  const points: string[] = [];
  for (let i = 0; i <= steps; i++) {
    const angle = (i / steps) * Math.PI * 2;
    const px = cx + rx * Math.cos(angle) + randomOffset(seed, i * 2, r);
    const py = cy + ry * Math.sin(angle) + randomOffset(seed, i * 2 + 1, r);
    points.push(`${i === 0 ? 'M' : 'L'}${px.toFixed(1)},${py.toFixed(1)}`);
  }
  points.push('Z');
  return points.join(' ');
}

export function excalidrawToSvg(json: string, maxWidth: number = 800): string {
  let file: ExcalidrawFile;
  try {
    file = JSON.parse(json);
  } catch {
    return '<div style="padding:12px;color:#8888a0;font-size:13px;">Invalid Excalidraw file</div>';
  }

  const elements = file.elements || [];
  if (elements.length === 0) {
    return '<div style="padding:12px;color:#8888a0;font-size:13px;">Empty Excalidraw diagram</div>';
  }

  const bgColor = file.appState?.viewBackgroundColor || '#ffffff';

  // Build element map
  const elementMap = new Map<string, ExcalidrawElement>();
  for (const el of elements) {
    elementMap.set(el.id, el);
  }

  // Calculate bounding box
  let mnx = Infinity, mny = Infinity, mxx = -Infinity, mxy = -Infinity;
  for (const el of elements) {
    mnx = Math.min(mnx, el.x);
    mny = Math.min(mny, el.y);
    mxx = Math.max(mxx, el.x + (el.width || 0));
    mxy = Math.max(mxy, el.y + (el.height || 0));
  }
  if (mnx === Infinity) { mnx = 0; mny = 0; mxx = 400; mxy = 300; }

  const padding = 20;
  const width = mxx - mnx + padding * 2;
  const height = mxy - mny + padding * 2;
  const scale = Math.min(1, maxWidth / width);

  function renderEl(el: ExcalidrawElement): string {
    const seed = roughSeed(el.id);
    const roughness = el.roughness ?? 1;
    const stroke = el.strokeColor || '#1e1e1e';
    const fill = el.backgroundColor && el.backgroundColor !== 'transparent' ? el.backgroundColor : '';
    const sw = el.strokeWidth || 2;
    const opacity = (el.opacity ?? 100) / 100;

    switch (el.type) {
      case 'rectangle': {
        const path = roughRect(el.x, el.y, el.width, el.height, seed, roughness);
        let fillPath = '';
        if (fill) {
          fillPath = `<path d="${roughRect(el.x, el.y, el.width, el.height, seed + 100, roughness)}" fill="${fill}" stroke="none"/>`;
        }
        return `<g opacity="${opacity}">${fillPath}<path d="${path}" fill="none" stroke="${stroke}" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round"/></g>`;
      }
      case 'ellipse': {
        const cx = el.x + el.width / 2;
        const cy = el.y + el.height / 2;
        const path = roughEllipse(cx, cy, el.width, el.height, seed, roughness);
        let fillPath = '';
        if (fill) {
          fillPath = `<path d="${roughEllipse(cx, cy, el.width, el.height, seed + 100, roughness)}" fill="${fill}" stroke="none"/>`;
        }
        return `<g opacity="${opacity}">${fillPath}<path d="${path}" fill="none" stroke="${stroke}" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round"/></g>`;
      }
      case 'diamond': {
        const cx = el.x + el.width / 2;
        const cy = el.y + el.height / 2;
        const path = `${roughLine(cx, el.y, el.x + el.width, cy, seed * 2, roughness)} ${roughLine(el.x + el.width, cy, cx, el.y + el.height, seed * 3, roughness)} ${roughLine(cx, el.y + el.height, el.x, cy, seed * 4, roughness)} ${roughLine(el.x, cy, cx, el.y, seed * 5, roughness)}`;
        let fillPath = '';
        if (fill) {
          fillPath = `<path d="M${cx},${el.y} L${el.x + el.width},${cy} L${cx},${el.y + el.height} L${el.x},${cy} Z" fill="${fill}" stroke="none"/>`;
        }
        return `<g opacity="${opacity}">${fillPath}<path d="${path}" fill="none" stroke="${stroke}" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round"/></g>`;
      }
      case 'text': {
        const fontSize = el.fontSize || 20;
        const fontFamily = el.fontFamily === 1 ? 'Virgil, Segoe UI Emoji, sans-serif' :
                          el.fontFamily === 3 ? 'Cascadia, monospace' : 'Helvetica, Arial, sans-serif';
        const container = el.containerId ? elementMap.get(el.containerId) : null;
        let tx = el.x;
        let ty = el.y;
        if (container) {
          tx = container.x + container.width / 2;
          ty = container.y + container.height / 2;
        }
        const anchor = container ? 'middle' : (el.textAlign === 'center' ? 'middle' : el.textAlign === 'right' ? 'end' : 'start');
        const dominantBaseline = container ? 'central' : 'auto';
        const text = (el.text || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
        return `<text x="${tx}" y="${ty}" fill="${stroke}" font-size="${fontSize}" font-family="${fontFamily}" text-anchor="${anchor}" dominant-baseline="${dominantBaseline}" opacity="${opacity}">${text}</text>`;
      }
      case 'arrow':
      case 'line': {
        const points = el.points || [[0, 0], [el.width, el.height]];
        let path = '';
        for (let i = 1; i < points.length; i++) {
          const [px0, py0] = points[i - 1];
          const [px1, py1] = points[i];
          path += roughLine(el.x + px0, el.y + py0, el.x + px1, el.y + py1, seed + i, roughness);
        }
        const markerEnd = el.type === 'arrow' && el.endArrowhead !== null ? ' marker-end="url(#arrowhead)"' : '';
        return `<path d="${path}" fill="none" stroke="${stroke}" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round" opacity="${opacity}"${markerEnd}/>`;
      }
      default:
        return '';
    }
  }

  const elementSvg = elements.map(renderEl).join('\n');

  return `<div style="margin:12px 0"><svg width="${width * scale}" height="${height * scale}" viewBox="${mnx - padding} ${mny - padding} ${width} ${height}" style="border-radius:8px;border:1px solid #2a2a3a;max-width:100%"><defs><marker id="arrowhead" markerWidth="10" markerHeight="7" refX="10" refY="3.5" orient="auto"><polygon points="0 0, 10 3.5, 0 7" fill="#1e1e1e"/></marker></defs><rect x="${mnx - padding}" y="${mny - padding}" width="${width}" height="${height}" fill="${bgColor}"/>${elementSvg}</svg></div>`;
}
