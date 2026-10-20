// JSON in and out of the API.
//
// Every number the API carries (money in cents, edit sequences) fits a JS number
// exactly, and the generated types say so, so plain JSON does.

export function parseApiJson<T>(text: string): T {
  return JSON.parse(text) as T;
}

export function stringifyApiJson(value: unknown): string {
  return JSON.stringify(value);
}

/** Deep copy. */
export function clone<T>(value: T): T {
  return structuredClone(value);
}
