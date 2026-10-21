// Report export formats as the server's `POST /reports/export` knows them:
// the generated `ExportFormat` names them (Csv | Xlsx | Pdf); the query string
// takes them lower-cased, with `paper=letter|a4` for the PDF. The labels, file
// extensions and MIME types the UI and the fixture share live here. Pure.

import type { ExportPaper } from '~/api/client';
import type { ExportFormat } from '~/api/types';

export type { ExportPaper };

export const EXPORT_FORMATS: readonly ExportFormat[] = ['Xlsx', 'Pdf', 'Csv'];
export const EXPORT_PAPERS: readonly ExportPaper[] = ['letter', 'a4'];

export interface ExportFormatInfo {
  /** What the menu says. */
  label: string;
  /** `Content-Disposition`'s extension. */
  ext: string;
  mime: string;
}

export const EXPORT_FORMAT_INFO: Record<ExportFormat, ExportFormatInfo> = {
  Xlsx: { label: 'Excel workbook', ext: 'xlsx', mime: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' },
  Pdf: { label: 'PDF', ext: 'pdf', mime: 'application/pdf' },
  Csv: { label: 'CSV', ext: 'csv', mime: 'text/csv;charset=utf-8' },
};

export const EXPORT_PAPER_LABEL: Record<ExportPaper, string> = { letter: 'Letter', a4: 'A4' };

/** The `?format=` value for a format. */
export function formatQuery(format: ExportFormat): string {
  return format.toLowerCase();
}

export function isExportFormat(v: unknown): v is ExportFormat {
  return typeof v === 'string' && Object.hasOwn(EXPORT_FORMAT_INFO, v);
}

export function isExportPaper(v: unknown): v is ExportPaper {
  return v === 'letter' || v === 'a4';
}

/** The file name the server would give: the title slugged, the format's extension. */
export function exportFileName(title: string, format: ExportFormat): string {
  const slug = title.replace(/[^\w -]+/g, '').trim();
  return `${slug || 'report'}.${EXPORT_FORMAT_INFO[format].ext}`;
}
