// Renders every open window, showing only the active one. Hidden windows stay
// mounted, so switching back finds the half-written check exactly as it was.
// Each window has its own error boundary: one broken screen doesn't take the
// others (and their unsaved work) down with it.

import { Alert, Button, Stack, Text } from '@mantine/core';
import { Component, type ReactNode } from 'react';
import { NotFound } from '~/screens/Placeholder';
import { WINDOW_KINDS } from '~/windows/catalog';
import { WindowProvider, useWindows } from '~/windows/WindowsProvider';
import classes from './shell.module.css';

class WindowErrorBoundary extends Component<{ children: ReactNode; onClose(): void }, { error: Error | null }> {
  override state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  override render() {
    if (!this.state.error) return this.props.children;
    return (
      <Stack p="md">
        <Alert color="red" title="This window hit a problem">
          <Text size="sm" mb="sm">
            {this.state.error.message}
          </Text>
          <Button size="xs" variant="light" color="red" onClick={this.props.onClose}>
            Close window
          </Button>
        </Alert>
      </Stack>
    );
  }
}

export function WindowHost() {
  const { state, notFound, close } = useWindows();
  return (
    <div className={classes.host}>
      {notFound && (
        <div className={classes.window}>
          <NotFound />
        </div>
      )}
      {state.windows.map((desc) => {
        const active = desc.key === state.active && !notFound;
        return (
          <div key={desc.key} className={classes.window} hidden={!active} data-window={desc.key} aria-hidden={!active}>
            <WindowProvider desc={desc} isActive={active}>
              <WindowErrorBoundary onClose={() => close(desc.key, { force: true })}>{WINDOW_KINDS[desc.kind].render(desc)}</WindowErrorBoundary>
            </WindowProvider>
          </div>
        );
      })}
    </div>
  );
}
