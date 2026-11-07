// Avalonia size box: 200 x 200 device-independent pixels, magenta on white.
// FreeBSD needs what BsdProbe found (2026-11-04): Avalonia's platform
// detection configures nothing, so X11, Skia and HarfBuzz are set
// explicitly; and Avalonia.X11 calls epoll in "libc", so "libc" is mapped
// to libc-epoll.so (an empty library linked to epoll-shim), copied next to
// the program with our libSkiaSharp 119 and a libHarfBuzzSharp symlink.
using System.Runtime.InteropServices;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Media;

class App : Application
{
    // A window needs a theme for its template; no XAML in this program.
    public override void Initialize() => Styles.Add(new Avalonia.Themes.Fluent.FluentTheme());

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime d)
        {
            var box = new Border { Width = 200, Height = 200, Background = new SolidColorBrush(Color.FromRgb(255, 0, 255)) };
            var canvas = new Canvas { Width = 240, Height = 240, Background = Brushes.White };
            Canvas.SetLeft(box, 20);
            Canvas.SetTop(box, 20);
            canvas.Children.Add(box);
            d.MainWindow = new Window
            {
                Title = "vergence-box-avalonia",
                Content = canvas,
                SizeToContent = SizeToContent.WidthAndHeight,
                CanResize = false,
            };
        }
        base.OnFrameworkInitializationCompleted();
    }
}

static class Program
{
    [STAThread]
    static int Main(string[] args)
    {
        var glue = Path.Combine(AppContext.BaseDirectory, "libc-epoll.so");
        if (OperatingSystem.IsFreeBSD() && File.Exists(glue))
            NativeLibrary.SetDllImportResolver(typeof(X11PlatformOptions).Assembly,
                (lib, _, _) => lib == "libc" ? NativeLibrary.Load(glue) : IntPtr.Zero);
        var b = AppBuilder.Configure<App>();
        b = OperatingSystem.IsFreeBSD() ? b.UseX11().UseSkia().UseHarfBuzz() : b.UsePlatformDetect();
        return b.StartWithClassicDesktopLifetime(args);
    }
}
