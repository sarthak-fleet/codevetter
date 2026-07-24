import type { AgentProvider } from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

export function AgentProviderMark({
  provider,
  className,
}: {
  provider: AgentProvider;
  className?: string;
}) {
  if (provider === 'claude') {
    return (
      <svg
        aria-hidden="true"
        data-agent-provider-mark="claude"
        viewBox="0 0 24 24"
        className={cn('h-4 w-4 text-[#d97757]', className)}
        fill="none"
      >
        <path
          d="M12 3.25v17.5M3.25 12h17.5M5.81 5.81l12.38 12.38M18.19 5.81 5.81 18.19M8.65 3.9l6.7 16.2M3.9 8.65l16.2 6.7M15.35 3.9l-6.7 16.2M20.1 8.65l-16.2 6.7"
          stroke="currentColor"
          strokeWidth="1.65"
          strokeLinecap="round"
        />
      </svg>
    );
  }

  return (
    <svg
      aria-hidden="true"
      data-agent-provider-mark="codex"
      viewBox="0 0 24 24"
      className={cn('h-4 w-4 text-emerald-300', className)}
      fill="none"
    >
      <path
        d="M12 3.4 16.1 5.75v4.72L12 12.82 7.9 10.47V5.75L12 3.4Zm4.1 2.35 4.08 2.36v4.71l-4.08 2.36L12 12.82v-4.7l4.1-2.37Zm4.08 7.07v4.72L16.1 19.9 12 17.54v-4.72l4.1-2.35 4.08 2.35Zm-4.08 7.08L12 22.25 7.9 19.9v-4.72l4.1-2.36 4.1 2.36v4.72ZM7.9 19.9l-4.08-2.36v-4.72l4.08-2.35 4.1 2.35v4.72L7.9 19.9Zm-4.08-7.08V8.11L7.9 5.75 12 8.11v4.71l-4.1 2.36-4.08-2.36Z"
        stroke="currentColor"
        strokeWidth="1.35"
        strokeLinejoin="round"
      />
    </svg>
  );
}
