using System.Windows;
using DynamicIsland.Core;
using DynamicIsland.Settings;
using DynamicIsland.UI;
using Windows.System.Power;

namespace DynamicIsland.Addons.SystemAddons;

/// <summary>(3) Charger plugged in / unplugged with the charge level, and warnings when the battery runs low.</summary>
public sealed class BatteryAddon : Addon
{
    private bool _pluggedIn;
    private int _lastPercent;

    public BatteryAddon(AddonContext context) : base(context) { }

    public override string Key => nameof(AddonSettings.Battery);
    public override string Name => "Battery";
    protected override bool IsEnabled(AddonSettings o) => o.Battery;

    public override void Preview() => Indicate(82, charging: true);

    protected override void Start()
    {
        if (PowerManager.BatteryStatus == BatteryStatus.NotPresent)
        {
            SetStatus(L.T("This PC has no battery."), problem: false);
            return; // desktop PC: nothing to show
        }
        _pluggedIn = IsPluggedIn;
        _lastPercent = PowerManager.RemainingChargePercent;
        PowerManager.PowerSupplyStatusChanged += OnPowerSupply;
        PowerManager.RemainingChargePercentChanged += OnPercent;
    }

    protected override void Stop()
    {
        PowerManager.PowerSupplyStatusChanged -= OnPowerSupply;
        PowerManager.RemainingChargePercentChanged -= OnPercent;
    }

    private static bool IsPluggedIn => PowerManager.PowerSupplyStatus != PowerSupplyStatus.NotPresent;

    private void OnPowerSupply(object? sender, object e) => Context.Ui(() =>
    {
        var plugged = IsPluggedIn;
        if (plugged == _pluggedIn) return;
        _pluggedIn = plugged;
        var percent = PowerManager.RemainingChargePercent;

        if (plugged)
        {
            Indicate(percent, charging: true);
        }
        else
        {
            Indicate(percent, charging: false);
        }
    });

    private void OnPercent(object? sender, object e) => Context.Ui(() =>
    {
        var percent = PowerManager.RemainingChargePercent;
        var previous = _lastPercent;
        _lastPercent = percent;
        if (_pluggedIn || percent >= previous) return;

        // Warn once when crossing each threshold on the way down.
        if (previous > Options.BatteryCriticalPercent && percent <= Options.BatteryCriticalPercent)
        {
            Announce(L.T("Battery very low"), L.F("{0}% left. Plug in your charger.", percent), percent, Ui.Red, false, urgent: true);
        }
        else if (previous > Options.BatteryLowPercent && percent <= Options.BatteryLowPercent)
        {
            Announce(L.T("Low battery"), L.F("{0}% left", percent), percent, Ui.Amber, false);
        }
    });

    /// <summary>
    /// The iPhone's charging indicator: the island widens with "Charging" on the leading side and the percentage with a
    /// battery on the trailing side, green while charging (red when low).
    /// </summary>
    private void Indicate(int percent, bool charging)
    {
        var color = percent <= Options.BatteryLowPercent ? Ui.Red : charging ? Ui.Green : Ui.Primary;
        var label = Ui.Text(charging ? L.T("Charging") : L.T("On battery"), 15, FontWeights.SemiBold, Ui.Primary);
        var level = Ui.Text($"{percent}%", 15, FontWeights.SemiBold, color == Ui.Primary ? Ui.Primary : color, tabular: true);
        level.Margin = new Thickness(0, 0, 7, 0);
        var battery = new System.Windows.Controls.StackPanel
        {
            Orientation = System.Windows.Controls.Orientation.Horizontal,
            Children = { level, Icons.Battery(percent / 100.0, charging, color, 27) },
        };
        Show(new IslandAlert
        {
            Content = Ui.Indicator(label, battery),
            Size = new Size(Ui.IndicatorWidth, Ui.IndicatorHeight),
            Duration = TimeSpan.FromSeconds(2.5),
            Keyline = color == Ui.Primary ? null : color.Color,
        });
    }

    private void Announce(string title, string subtitle, int percent, System.Windows.Media.Brush color, bool charging, bool urgent = false)
    {
        var icon = new System.Windows.Controls.Border
        {
            Width = Ui.IconSize, Height = Ui.IconSize,
            Child = Icons.Battery(percent / 100.0, charging, color, 34),
        };
        Show(new IslandAlert
        {
            Content = Ui.AlertView(icon, title, subtitle, color == Ui.Primary ? null : color),
            Duration = TimeSpan.FromSeconds(urgent ? 8 : 3.5),
            StickyUntilHover = urgent,
            BreaksThroughQuiet = urgent,
            Keyline = (color as System.Windows.Media.SolidColorBrush)?.Color,
        });
    }
}
