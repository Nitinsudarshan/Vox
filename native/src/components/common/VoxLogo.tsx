import React from 'react';
import logoLight from '@/assets/vox-logo-light.png';
import logoDark from '@/assets/vox-logo-dark.png';

interface VoxLogoProps {
  className?: string;
}

/**
 * Vox brand logo using the official brand asset PNGs.
 * Switches between light and dark variants based on the document's
 * current dark-mode class (which Tauri's root `<html class="dark">` sets).
 */
export const VoxLogo: React.FC<VoxLogoProps> = ({ className = 'w-6 h-6' }) => {
  const [isDark, setIsDark] = React.useState(
    () => document.documentElement.classList.contains('dark')
  );

  React.useEffect(() => {
    const observer = new MutationObserver(() => {
      setIsDark(document.documentElement.classList.contains('dark'));
    });
    observer.observe(document.documentElement, { attributeFilter: ['class'] });
    return () => observer.disconnect();
  }, []);

  return (
    <img
      src={isDark ? logoDark : logoLight}
      alt="Vox logo"
      className={className}
      draggable={false}
    />
  );
};
