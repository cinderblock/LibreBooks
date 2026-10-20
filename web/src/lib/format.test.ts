import { describe, expect, test } from 'bun:test';
import { describeDuration, durationToMinutes, formatBytes, minutesToDuration } from './format';

describe('formatBytes', () => {
  test('units', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(20 * 1024)).toBe('20 KB');
    expect(formatBytes(5_300_000)).toBe('5.1 MB');
    expect(formatBytes(3 * 1024 ** 3)).toBe('3.00 GB');
  });
});

describe('ISO 8601 durations', () => {
  test('parse', () => {
    expect(durationToMinutes('PT6H')).toBe(360);
    expect(durationToMinutes('P1D')).toBe(1440);
    expect(durationToMinutes('PT90M')).toBe(90);
    expect(durationToMinutes('P1DT12H')).toBe(2160);
    expect(durationToMinutes('PT30S')).toBe(0.5);
    expect(durationToMinutes('6 hours')).toBeNull();
    expect(durationToMinutes('P')).toBeNull();
    expect(durationToMinutes('')).toBeNull();
    expect(durationToMinutes(null)).toBeNull();
  });
  test('format picks the shortest unit', () => {
    expect(minutesToDuration(1440)).toBe('P1D');
    expect(minutesToDuration(360)).toBe('PT6H');
    expect(minutesToDuration(90)).toBe('PT90M');
    expect(minutesToDuration(0)).toBe('PT1M');
  });
  test('round trip', () => {
    for (const iso of ['PT6H', 'P2D', 'PT45M', 'PT12H']) expect(minutesToDuration(durationToMinutes(iso)!)).toBe(iso);
  });
  test('describe', () => {
    expect(describeDuration('PT6H')).toBe('every 6 hours');
    expect(describeDuration('PT1H')).toBe('every hour');
    expect(describeDuration('P1D')).toBe('every day');
    expect(describeDuration('PT15M')).toBe('every 15 minutes');
    expect(describeDuration('weird')).toBe('weird');
  });
});
