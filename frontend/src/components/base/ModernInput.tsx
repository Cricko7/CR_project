import { forwardRef, useId, type InputHTMLAttributes } from 'react';
import { cn } from '../../lib/cn';

export interface ModernInputProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, 'className' | 'size'> {
  className?: string;
  inputClassName?: string;
  label: string;
}

export const ModernInput = forwardRef<HTMLInputElement, ModernInputProps>(
  ({ className, id, inputClassName, label, placeholder = ' ', type = 'text', ...props }, ref) => {
    const generatedId = useId();
    const inputId = id ?? generatedId;

    return (
      <div className={cn('group relative', className)}>
        <input
          ref={ref}
          id={inputId}
          type={type}
          placeholder={placeholder}
          className={cn(
            'peer w-full rounded-2xl border border-white/10 bg-white/5 px-4 pb-3 pt-6 text-sm text-white',
            'outline-none backdrop-blur-xl transition-colors duration-200 placeholder:text-transparent',
            'focus:border-cyan-300/60',
            inputClassName
          )}
          {...props}
        />
        <label
          htmlFor={inputId}
          className={cn(
            'pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-sm text-white/55',
            'transition-all duration-200',
            'peer-focus:top-3 peer-focus:translate-y-0 peer-focus:text-xs peer-focus:text-cyan-200',
            'peer-[&:not(:placeholder-shown)]:top-3 peer-[&:not(:placeholder-shown)]:translate-y-0',
            'peer-[&:not(:placeholder-shown)]:text-xs peer-[&:not(:placeholder-shown)]:text-white/70'
          )}
        >
          {label}
        </label>
        <div
          aria-hidden="true"
          className={cn(
            'pointer-events-none absolute inset-0 rounded-2xl ring-1 ring-inset ring-white/5 transition-opacity duration-300',
            'opacity-0 shadow-[0_0_0_1px_rgba(125,211,252,0.55),0_0_34px_rgba(14,165,233,0.28)]',
            'peer-focus:opacity-100'
          )}
        />
      </div>
    );
  }
);

ModernInput.displayName = 'ModernInput';
