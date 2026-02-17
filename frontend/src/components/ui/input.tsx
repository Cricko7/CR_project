import * as React from 'react';
import { cn } from '../../lib/cn';

export const Input = React.forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(
  ({ className, type = 'text', ...props }, ref) => (
    <input
      ref={ref}
      type={type}
      className={cn(
        'flex h-10 w-full rounded-md border border-white/15 bg-slate-900/70 px-3 py-2 text-sm text-slate-100',
        'placeholder:text-slate-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-300/70',
        'disabled:cursor-not-allowed disabled:opacity-50',
        className
      )}
      {...props}
    />
  )
);

Input.displayName = 'Input';
