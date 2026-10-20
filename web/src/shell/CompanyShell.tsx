// Everything under `/c/:companyId`: the company's context, its live-update
// stream, the window manager, global shortcuts, and the desktop or phone shell.

import { useDocumentTitle } from '@mantine/hooks';
import { Navigate, useParams } from 'react-router';
import { CompanyProvider, useCompanyEvents, useCompanyId } from '~/api/context';
import { useCompanyInfo } from '~/api/hooks';
import { QuickCreateProvider } from '~/inputs/QuickCreate';
import { useIsMobile } from '~/lib/useMobile';
import { windowTitle } from '~/windows/catalog';
import { WindowsProvider, useWindows } from '~/windows/WindowsProvider';
import { CommandPalette } from './CommandPalette';
import { DesktopShell } from './DesktopShell';
import { GlobalDialogs, useGlobalCommands } from './GlobalCommands';
import { MobileShell } from './MobileShell';

export function CompanyShell() {
  const { companyId } = useParams();
  if (!companyId) return <Navigate to="/" replace />;
  return (
    <CompanyProvider companyId={companyId}>
      <WindowsProvider base={`/c/${encodeURIComponent(companyId)}`}>
        <QuickCreateProvider>
          <ShellInner />
        </QuickCreateProvider>
      </WindowsProvider>
    </CompanyProvider>
  );
}

function ShellInner() {
  const co = useCompanyId();
  const live = useCompanyEvents(co);
  const cmds = useGlobalCommands();
  const mobile = useIsMobile();
  const { state } = useWindows();
  const { data: info } = useCompanyInfo();
  const active = state.windows.find((w) => w.key === state.active);
  useDocumentTitle([active ? windowTitle(active, state.titles) : null, info?.name, 'LibreBooks'].filter(Boolean).join(' · '));
  const onShortcuts = () => cmds.setOverlay(true);
  return (
    <>
      {mobile ? <MobileShell live={live} onShortcuts={onShortcuts} /> : <DesktopShell live={live} run={cmds.run} onShortcuts={onShortcuts} />}
      <CommandPalette run={cmds.run} />
      <GlobalDialogs {...cmds} />
    </>
  );
}
