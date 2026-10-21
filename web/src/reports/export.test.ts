import { describe, expect, test } from 'bun:test';
import { EXPORT_FORMATS, exportFileName, formatQuery, isExportFormat, isExportPaper } from './export';

describe('report export formats', () => {
  test('the query spells the generated enum lower-cased', () => {
    expect(EXPORT_FORMATS.map(formatQuery)).toEqual(['xlsx', 'pdf', 'csv']);
  });

  test('guards accept the server\'s values only', () => {
    expect(isExportFormat('Pdf')).toBe(true);
    expect(isExportFormat('pdf')).toBe(false);
    expect(isExportFormat('toString')).toBe(false);
    expect(isExportPaper('a4')).toBe(true);
    expect(isExportPaper('A4')).toBe(false);
  });

  test('file names slug the title and take the extension', () => {
    expect(exportFileName('Profit & Loss', 'Xlsx')).toBe('Profit  Loss.xlsx');
    expect(exportFileName('***', 'Pdf')).toBe('report.pdf');
  });
});
