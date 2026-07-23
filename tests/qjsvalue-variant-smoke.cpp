#include <QCoreApplication>
#include <QFile>
#include <QJSEngine>
#include <QJSValue>
#include <QMetaType>
#include <QTextStream>
#include <QVariant>

namespace {
bool expectType(QJSEngine &engine, const QString &expression, QMetaType::Type expected)
{
    const QJSValue value = engine.evaluate(expression);
    if (value.isError()) {
        QTextStream(stderr) << expression << ": " << value.toString() << '\n';
        return false;
    }
    const QVariant variant = value.toVariant();
    if (variant.metaType().id() != expected) {
        QTextStream(stderr) << expression << " produced " << variant.metaType().name()
                            << ", expected " << QMetaType(expected).name() << '\n';
        return false;
    }
    return true;
}

} // namespace

int main(int argc, char **argv)
{
    QCoreApplication application(argc, argv);
    if (argc != 2) {
        QTextStream(stderr) << "usage: qjsvalue-variant-smoke <main.js>\n";
        return 2;
    }

    QFile sourceFile(QString::fromLocal8Bit(argv[1]));
    if (!sourceFile.open(QIODevice::ReadOnly | QIODevice::Text)) {
        QTextStream(stderr) << sourceFile.errorString() << '\n';
        return 2;
    }

    QJSEngine engine;
    const QString prelude = QStringLiteral(R"JS(
        var workspace = {
            activeWindow: null,
            stackingOrder: [],
            windowAdded: { connect: function () {} },
            windowActivated: { connect: function () {} }
        };
        function callDBus() {}
        function registerShortcut() {}
        function QTimer() {
            this.interval = 0;
            this.timeout = { connect: function () {} };
            this.start = function () {};
        }
    )JS");
    const QJSValue setup = engine.evaluate(prelude);
    const QJSValue loaded = engine.evaluate(QString::fromUtf8(sourceFile.readAll()), argv[1]);
    if (setup.isError() || loaded.isError()) {
        QTextStream(stderr) << (setup.isError() ? setup.toString() : loaded.toString()) << '\n';
        return 1;
    }

    const bool integersAreInt32 =
        expectType(engine, QStringLiteral("finiteInteger(42, 1, 2147483647)"), QMetaType::Int)
        && expectType(engine, QStringLiteral("finiteInteger(-100.4, -2147483648, 2147483647)"), QMetaType::Int)
        && expectType(engine, QStringLiteral("finiteInteger(1920.2, 1, 2147483647)"), QMetaType::Int);
    if (!integersAreInt32) {
        return 1;
    }

    QTextStream(stdout) << "QJSEngine int32 coercion smoke test passed\n";
    return 0;
}
