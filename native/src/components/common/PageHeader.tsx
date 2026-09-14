import React from 'react';
import { Badge } from '@/components/ui/badge';
import { LucideIcon } from 'lucide-react';
import { cn } from '@/lib/utils';

export interface PageHeaderBadge {
  label: string;
  icon?: LucideIcon;
  variant?: 'emerald' | 'default' | 'purple' | 'amber' | 'outline' | 'secondary' | 'destructive';
}

export interface PageHeaderProps {
  kicker?: string;
  badge?: PageHeaderBadge;
  /**
   * The page's name. A node rather than a string so a detail page can put an
   * inline rename field where its title is, rather than beside it.
   */
  title: React.ReactNode;
  highlightText?: string;
  description?: string | React.ReactNode;
  variant?: 'banner' | 'minimal';
  glowColor?: 'emerald' | 'primary' | 'purple' | 'amber' | 'none';
  /**
   * Rendered at the far left, before the title block.
   *
   * For a control that belongs to the banner rather than beside it — a back
   * button on a detail page, which has nowhere else to go once the banner is
   * the page's own header.
   */
  leading?: React.ReactNode;
  /**
   * Rendered full width underneath the title row, inside the banner.
   *
   * For something that needs the whole width rather than the right-hand
   * corner, such as a meeting's audio player.
   */
  footer?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
  compact?: boolean;
}

export const PageHeader: React.FC<PageHeaderProps> = ({
  kicker,
  badge,
  title,
  highlightText,
  description,
  variant = 'banner',
  glowColor = 'none',
  leading,
  footer,
  children,
  className,
  compact = false,
}) => {
  if (variant === 'minimal') {
    return (
      <div className={cn("space-y-1 mb-3.5 shrink-0", className)}>
        {kicker && (
          <p className="font-mono text-[9px] font-semibold text-muted-foreground uppercase tracking-widest mb-1">
            {kicker}
          </p>
        )}
        {badge && (
          <div className="flex items-center gap-2 mb-1">
            <Badge
              variant={badge.variant || 'outline'}
              className="text-[9px] font-mono uppercase tracking-wider gap-1.5 py-0.5 px-2"
            >
              {badge.icon && <badge.icon className="w-2.5 h-2.5" />}
              <span>{badge.label}</span>
            </Badge>
          </div>
        )}
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5">
          {leading && <div className="flex items-center shrink-0">{leading}</div>}
          <div>
            <h1 className="text-lg md:text-xl font-extrabold tracking-tight text-foreground">
              {title}
              {highlightText && (
                <>
                  {' '}
                  <span className="italic text-primary">{highlightText}</span>
                </>
              )}
            </h1>
            {description && (
              <p className="text-[11px] text-muted-foreground max-w-xl leading-snug mt-0.5">
                {description}
              </p>
            )}
          </div>
          {children && <div className="flex items-center gap-2 shrink-0">{children}</div>}
        </div>
        {footer && <div className="mt-2.5">{footer}</div>}
      </div>
    );
  }

  const glowClass = {
    emerald: 'bg-emerald-500/10 to-emerald-500/5',
    primary: 'bg-primary/10 to-primary/5',
    purple: 'bg-purple-500/10 to-purple-500/5',
    amber: 'bg-amber-500/10 to-amber-500/5',
    none: 'bg-transparent',
  }[glowColor];

  return (
    <div
      className={cn(
        "relative rounded-lg border border-border bg-gradient-to-br from-card via-card/95 to-card/90 shadow-xs overflow-hidden shrink-0",
        compact ? "py-3 px-4 md:py-3.5 md:px-5 mb-3" : "py-3.5 px-4 md:py-4 md:px-5 mb-3.5",
        className
      )}
    >
      {glowColor !== 'none' && (
        <div
          className={cn(
            "absolute -right-8 -top-8 rounded-full pointer-events-none",
            compact ? "w-28 h-28 blur-2xl" : "w-36 h-36 blur-3xl",
            glowClass
          )}
        />
      )}

      <div
        className={cn(
          "relative z-10 flex flex-col md:flex-row md:items-center justify-between",
          compact ? "gap-3" : "gap-4"
        )}
      >
        {leading && <div className="flex items-center shrink-0 -ml-1.5">{leading}</div>}
        <div className={compact ? "space-y-1 min-w-0 flex-1" : "space-y-1.5 min-w-0 flex-1"}>
          {kicker && (
            <p className="font-mono text-[9px] font-semibold text-muted-foreground uppercase tracking-widest">
              {kicker}
            </p>
          )}

          {badge && (
            <div className="flex items-center gap-2">
              <Badge
                variant={badge.variant || 'outline'}
                className={cn(
                  "font-mono uppercase tracking-wider",
                  compact
                    ? "text-[9px] gap-1 py-0.5 px-1.5"
                    : "text-[10px] gap-1.5 py-0.5 px-2"
                )}
              >
                {badge.icon && <badge.icon className={compact ? "w-2.5 h-2.5" : "w-3 h-3"} />}
                <span>{badge.label}</span>
              </Badge>
            </div>
          )}

          <h1
            className={cn(
              "font-extrabold tracking-tight text-foreground",
              compact ? "text-lg md:text-xl" : "text-xl md:text-2xl"
            )}
          >
            {title}
            {highlightText && (
              <>
                {' '}
                <span className="italic text-primary">{highlightText}</span>
              </>
            )}
          </h1>

          {description && (
            <div
              className={cn(
                "text-muted-foreground",
                compact
                  ? "text-[11px] max-w-xl leading-snug"
                  : "text-xs max-w-2xl leading-relaxed"
              )}
            >
              {description}
            </div>
          )}
        </div>

        {children && <div className="flex items-center gap-2 shrink-0">{children}</div>}
      </div>

      {footer && <div className={cn("relative z-10", compact ? "mt-3" : "mt-3.5")}>{footer}</div>}
    </div>
  );
};
