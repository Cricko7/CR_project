import * as React from 'react';
import { cn } from '../../lib/cn';

type BadgeVariant = 'default' | 'secondary' | 'outline';

const variantClasses: Record<BadgeVariant, string> = {
  default: 'bg-cyan-500/20 text-cyan-200 border-cyan-300/40',
  secondary: 'bg-indigo-500/20 text-indigo-200 border-indigo-300/40',
  outline: 'bg-transparent text-slate-200 border-white/20'
};

export interface BadgeProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: BadgeVariant;
}

export const Badge = ({ className, variant = 'default', ...props }: BadgeProps) => (
  <div
    className={cn(
      'inline-flex items-center rounded-full border px-2.5 py-0.5 text-[11px] font-semibold uppercase tracking-wide',
      variantClasses[variant],
      className
    )}
    {...props}
  />
);
