import React from 'react';
import { AlertCircle } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';

interface CloseConfirmationDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirmClose: () => void;
}

/**
 * Confirmation dialog shown when the user clicks the window Close button or
 * initiates a window close request.
 *
 * Distinguishes completely quitting the desktop app from merely hiding it to
 * the background / tray.
 */
export const CloseConfirmationDialog: React.FC<CloseConfirmationDialogProps> = ({
  open,
  onOpenChange,
  onConfirmClose,
}) => {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[420px] bg-card border-border shadow-xl">
        <DialogHeader className="space-y-2">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-destructive/10 text-destructive flex items-center justify-center shrink-0">
              <AlertCircle className="w-4 h-4" />
            </div>
            <DialogTitle className="text-base font-semibold text-foreground">
              Close Vox?
            </DialogTitle>
          </div>
          <DialogDescription className="text-xs text-muted-foreground leading-relaxed pt-1">
            Are you sure you want to close Vox?
          </DialogDescription>
        </DialogHeader>

        <p className="text-xs text-muted-foreground/80 leading-relaxed -mt-1">
          Closing will completely exit the application and stop background voice capture.
          To keep Vox running silently in the background, use <strong className="text-foreground font-semibold">Hide</strong> instead.
        </p>

        <DialogFooter className="gap-2 sm:gap-2 pt-3 border-t border-border/50">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => onOpenChange(false)}
            className="text-xs h-8 px-3 cursor-pointer"
            autoFocus
          >
            Cancel
          </Button>
          <Button
            type="button"
            variant="destructive"
            size="sm"
            onClick={onConfirmClose}
            className="text-xs h-8 px-3.5 cursor-pointer font-medium"
          >
            Close Vox
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
