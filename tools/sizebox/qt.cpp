// Qt 6 size box: 200 x 200 device-independent pixels, magenta on white.
#include <QApplication>
#include <QPainter>
#include <QWidget>
struct Box : QWidget {
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.fillRect(rect(), Qt::white);
        p.fillRect(20, 20, 200, 200, QColor(255, 0, 255));
    }
};
int main(int argc, char **argv) {
    QApplication app(argc, argv);
    Box b;
    b.setWindowTitle("vergence-box-qt");
    b.setFixedSize(240, 240);
    b.show();
    return app.exec();
}
