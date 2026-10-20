// The central shortcut registry.
//
// Components register *handlers* for commands (not keys) with a scope; the
// keymap decides which keys mean which command. On a keydown the registry finds
// the commands whose keys match, then runs the most specific active handler
// (field > grid > form > window > global; among equals, the most recently
// registered, i.e. the innermost/newest window). A handler returning `false`
// declines and the next one is tried. Pure TS: no React, no DOM, unit-tested.

import { matches, parseCombo, type Combo, type KeyLike, type OS } from './combo';
import { COMMANDS, SCOPE_RANK, type CommandDef, type CommandId, type Scope } from './commands';
import type { Keymap } from './keymaps';

export type ShortcutHandler = (e: KeyLike) => boolean | void;

export interface Registration {
  command: CommandId;
  handler: ShortcutHandler;
  /** Defaults to the command's own scope. */
  scope?: Scope;
  /** Checked at dispatch time: e.g. "is my window the active one". */
  enabled?: () => boolean;
}

interface Entry extends Registration {
  scope: Scope;
  order: number;
}

export interface DispatchContext {
  /** Focus is in a text input, textarea, select or contenteditable. */
  inField: boolean;
}

/** Keys that type or move a caret: never steal them from a field. */
function isTypingCombo(c: Combo): boolean {
  if (c.ctrl || c.alt || c.meta) return false;
  return c.key.length === 1 || ['enter', 'tab', 'backspace', 'delete', 'home', 'end', 'arrowup', 'arrowdown', 'arrowleft', 'arrowright'].includes(c.key);
}

export class ShortcutRegistry {
  private entries: Entry[] = [];
  private order = 0;
  private parsed = new Map<CommandId, Combo[]>();
  private listeners = new Set<() => void>();

  constructor(
    private keymap: Keymap,
    readonly os: OS,
  ) {
    this.reparse();
  }

  setKeymap(keymap: Keymap) {
    this.keymap = keymap;
    this.reparse();
    this.notify();
  }

  getKeymap(): Keymap {
    return this.keymap;
  }

  private reparse() {
    this.parsed.clear();
    for (const [cmd, combos] of Object.entries(this.keymap) as Array<[CommandId, string[]]>) {
      this.parsed.set(cmd, combos.map((c) => parseCombo(c, this.os)));
    }
  }

  /** Register a handler; returns the unregister function. */
  register(r: Registration): () => void {
    const entry: Entry = { ...r, scope: r.scope ?? COMMANDS[r.command].scope, order: this.order++ };
    this.entries.push(entry);
    this.notify();
    return () => {
      this.entries = this.entries.filter((e) => e !== entry);
      this.notify();
    };
  }

  /** The commands an event's keys mean (before checking for handlers). */
  commandsFor(e: KeyLike, ctx: DispatchContext): CommandId[] {
    const out: CommandId[] = [];
    for (const [cmd, combos] of this.parsed) {
      const def: CommandDef = COMMANDS[cmd];
      for (const c of combos) {
        if (!matches(e, c)) continue;
        if (ctx.inField) {
          // Explicit opt-out (Ctrl+A select-all, Ctrl+Y redo) or a typing key.
          if (def.inFields === false) continue;
          if (isTypingCombo(c) && def.inFields !== true) continue;
        }
        out.push(cmd);
        break;
      }
    }
    return out;
  }

  /**
   * Run the best handler for this key. Returns the command that handled it, or
   * null (the caller then leaves the event alone).
   */
  dispatch(e: KeyLike, ctx: DispatchContext): CommandId | null {
    const commands = new Set(this.commandsFor(e, ctx));
    if (commands.size === 0) return null;
    const candidates = this.entries
      .filter((en) => commands.has(en.command) && (en.enabled?.() ?? true))
      .sort((a, b) => SCOPE_RANK[b.scope] - SCOPE_RANK[a.scope] || b.order - a.order);
    for (const c of candidates) {
      if (c.handler(e) !== false) return c.command;
    }
    return null;
  }

  /** Combos for a command as written in the keymap ("Mod+W"). */
  combos(command: CommandId): string[] {
    return this.keymap[command] ?? [];
  }

  /** Commands that currently have an enabled handler, for the overlay. */
  activeCommands(): Array<{ command: CommandId; scope: Scope }> {
    const seen = new Map<CommandId, Scope>();
    for (const e of this.entries) {
      if (!(e.enabled?.() ?? true)) continue;
      const prev = seen.get(e.command);
      if (prev === undefined || SCOPE_RANK[e.scope] > SCOPE_RANK[prev]) seen.set(e.command, e.scope);
    }
    return [...seen].map(([command, scope]) => ({ command, scope }));
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify() {
    for (const fn of this.listeners) fn();
  }
}
