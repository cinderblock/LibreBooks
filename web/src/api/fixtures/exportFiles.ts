// FIXTURE ONLY: stand-ins for the server's XLSX and PDF report exports, so the
// export menu's download path can be exercised without a server. The PDF is a
// real one-page document (the title block and the first rows in Helvetica, on
// the paper asked for); the "workbook" is the CSV bytes under the XLSX MIME
// type: a placeholder, not something Excel opens. lb-reports' writers are the
// truth (plans/reports.md, "Export: CSV, XLSX, PDF").

import type { ExportPaper } from '../client';
import type { Report } from '../types';
import { EXPORT_FORMAT_INFO } from '~/reports/export';
import { reportToCsv, valueText } from '~/reports/rows';

/** Page sizes in PDF points (1/72 in). */
const PAPER: Record<ExportPaper, { w: number; h: number }> = { letter: { w: 612, h: 792 }, a4: { w: 595, h: 842 } };
const MARGIN = 36;

/** A PDF string literal: parentheses and backslashes escaped; the standard fonts are WinAnsi, so anything outside Latin-1 prints as `?`. */
function pdfText(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/[^\x20-\xff]/g, '?').replace(/[\\()]/g, (c) => `\\${c}`);
}

export function placeholderPdf(report: Report, paper: ExportPaper): Blob {
  const { w, h } = PAPER[paper];
  const ops: string[] = [];
  let y = h - MARGIN - 14;
  const line = (text: string, size: number, bold = false, x = MARGIN) => {
    ops.push(`BT /${bold ? 'F2' : 'F1'} ${size} Tf ${x} ${y.toFixed(1)} Td (${pdfText(text)}) Tj ET`);
    y -= size * 1.35;
  };
  line(report.company_name, 11);
  line(report.title, 14, true);
  line(report.subtitle, 10);
  if (report.basis) line(`${report.basis} Basis`, 9);
  y -= 6;
  line(report.columns.map((c) => c.label).join('    '), 8, true);
  for (const row of report.rows) {
    if (y < MARGIN + 20) break;
    if (row.kind === 'Spacer') {
      y -= 6;
      continue;
    }
    const cells = row.cells.map((c) => valueText(c.value)).join('    ');
    line(cells, 8, row.kind === 'Header' || row.kind === 'Subtotal' || row.kind === 'Total', MARGIN + row.depth * 10);
  }
  y = MARGIN;
  line('LibreBooks fixture preview - not the server’s rendering', 7);
  const content = ops.join('\n');

  // Objects in order; the xref table needs each one's byte offset.
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${w} ${h}] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>',
    `<< /Length ${byteLength(content)} >>\nstream\n${content}\nendstream`,
  ];
  let out = '%PDF-1.4\n';
  const offsets: number[] = [];
  objects.forEach((body, i) => {
    offsets.push(byteLength(out));
    out += `${i + 1} 0 obj\n${body}\nendobj\n`;
  });
  const xref = byteLength(out);
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const o of offsets) out += `${String(o).padStart(10, '0')} 00000 n \n`;
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new Blob([latin1(out)], { type: EXPORT_FORMAT_INFO.Pdf.mime });
}

/** Not a workbook: the CSV bytes with the XLSX MIME type, enough for the download path. */
export function placeholderXlsx(report: Report): Blob {
  return new Blob([reportToCsv(report)], { type: EXPORT_FORMAT_INFO.Xlsx.mime });
}

/** The text is Latin-1 (see `pdfText`), so one byte per character. */
function byteLength(s: string): number {
  return s.length;
}

function latin1(s: string): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(new ArrayBuffer(s.length));
  for (let i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i) & 0xff;
  return bytes;
}
