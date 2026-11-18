using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using DynamicIsland.Core;
using DynamicIsland.Settings;
using DynamicIsland.UI;
using Microsoft.Win32;

namespace DynamicIsland.Addons.Alerts;

/// <summary>
/// (22) "Restart to finish updates" when Windows Update is waiting for a restart. Restart gives a one-minute
/// countdown that can still be cancelled; Later asks again in four hours.
/// </summary>
public sealed class WindowsUpdateAddon : Addon
{
    private DispatcherTimer? _poll;
    private DateTime _snoozedUntil;
    private bool _restarting;

    public WindowsUpdateAddon(AddonContext context) : base(context) { }

    public override string Key => nameof(AddonSettings.WindowsUpdate);
    public override string Name => "Windows Update";
    protected override bool IsEnabled(AddonSettings o) => o.WindowsUpdate;

    protected override void Start()
    {
        _poll = new DispatcherTimer { Interval = TimeSpan.FromMinutes(15) };
        _poll.Tick += (_, _) => Check();
        _poll.Start();
        // First look a little after startup, not while the PC is still busy signing in.
        Context.Dispatcher.BeginInvoke(async () =>
        {
            await Task.Delay(TimeSpan.FromMinutes(2));
            if (Running) Check();
        });
    }

    protected override void Stop()
    {
        _poll?.Stop();
        _poll = null;
    }

    private void Check()
    {
        if (_restarting || DateTime.UtcNow < _snoozedUntil || !RestartPending()) return;
        _snoozedUntil = DateTime.UtcNow.AddHours(4); // until the user picks, don't ask again soon
        Prompt(Restart);
    }

    /// <summary>The sample from the settings app has the same buttons, but Restart only closes it.</summary>
    public override void Preview() => Prompt(() => { });

    private void Prompt(Action onRestart)
    {
        IslandAlert? alert = null;
        var later = Ui.ActionButton(L.T("Later"), () => Context.Manager.DismissAlert(alert));
        var restart = Ui.ActionButton(L.T("Restart"), () =>
        {
            Context.Manager.DismissAlert(alert);
            onRestart();
        }, Ui.Blue);

        alert = new IslandAlert
        {
            Content = Ui.ActionAlertView(Ui.GlyphCircle("", Ui.Blue, Ui.IconSize, 22), L.T("Updates are ready"),
                                         L.T("Restart to finish installing"), later, restart),
            Size = new Size(Ui.AlertWidth, Ui.ActionAlertHeight),
            Keyline = Ui.Blue.Color,
            Duration = TimeSpan.FromSeconds(15),
            StickyUntilHover = true,
        };
        Show(alert);
    }

    private void Restart()
    {
        _restarting = true;
        Run("shutdown.exe", $"/r /t 60 /c \"{L.T("Restarting to finish Windows updates")}\"");

        IslandAlert? countdown = null;
        var cancel = Ui.ActionButton(L.T("Cancel"), () =>
        {
            Run("shutdown.exe", "/a");
            _restarting = false;
            Context.Manager.DismissAlert(countdown);
        }, Ui.Red);
        countdown = new IslandAlert
        {
            Content = Ui.ActionAlertView(Ui.GlyphCircle("", Ui.Blue, Ui.IconSize, 22), L.T("Restarting in 1 minute"),
                                         L.T("Save your work"), cancel),
            Size = new Size(Ui.AlertWidth, Ui.ActionAlertHeight),
            Keyline = Ui.Red.Color,
            Sticky = true,
            BreaksThroughQuiet = true,
        };
        Show(countdown);
    }

    /// <summary>Windows Update (or servicing) marks a pending restart with these registry keys.</summary>
    private static bool RestartPending()
    {
        try
        {
            using var wu = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired");
            using var cbs = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending");
            return wu is not null || cbs is not null;
        }
        catch
        {
            return false;
        }
    }

    private static void Run(string exe, string args)
    {
        try
        {
            Process.Start(new ProcessStartInfo(exe, args) { UseShellExecute = false, CreateNoWindow = true });
        }
        catch (Exception ex)
        {
            Log.Error($"Could not run {exe} {args}", ex);
        }
    }
}
