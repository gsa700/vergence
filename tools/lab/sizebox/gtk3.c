/* GTK 3 size box: 200 x 200 logical pixels, magenta on white. */
#include <gtk/gtk.h>
static gboolean draw(GtkWidget *w, cairo_t *cr, gpointer d) {
    cairo_set_source_rgb(cr, 1, 1, 1); cairo_paint(cr);
    cairo_set_source_rgb(cr, 1, 0, 1); cairo_rectangle(cr, 20, 20, 200, 200); cairo_fill(cr);
    return TRUE;
}
int main(int argc, char **argv) {
    gtk_init(&argc, &argv);
    GtkWidget *win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(win), "vergence-box-gtk3");
    GtkWidget *a = gtk_drawing_area_new();
    gtk_widget_set_size_request(a, 240, 240);
    g_signal_connect(a, "draw", G_CALLBACK(draw), NULL);
    gtk_container_add(GTK_CONTAINER(win), a);
    g_signal_connect(win, "destroy", G_CALLBACK(gtk_main_quit), NULL);
    gtk_widget_show_all(win);
    gtk_main();
    return 0;
}
