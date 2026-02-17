import * as React from 'react';
import { cn } from '../../lib/cn';

export interface SeparatorProps extends React.HTMLAttributes<HTMLDivElement> {
  orientation?: 'horizontal' | 'vertical';
}

export const Separator = ({ className, orientation = 'horizontal', ...props }: SeparatorProps) => (
  <div
    className={cn(
      'shrink-0 bg-white/10',
      orientation === 'horizontal' ? 'h-px w-full' : 'h-full w-px',
      className
    )}
    {...props}
  />
);
