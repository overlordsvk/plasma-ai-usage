import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    property alias cfg_intervalMinutes: interval.value
    property alias cfg_opacityPercent: opacity.value
    property alias cfg_showBackground: background.checked
    property alias cfg_pinned: pinned.checked

    Kirigami.FormLayout {
        QQC2.SpinBox {
            id: interval
            Kirigami.FormData.label: "Update every (minutes):"
            from: 1
            to: 120
        }
        RowLayout {
            Kirigami.FormData.label: "Opacity:"
            QQC2.Slider {
                id: opacity
                from: 20
                to: 100
                stepSize: 5
                Layout.preferredWidth: Kirigami.Units.gridUnit * 10
            }
            QQC2.Label { text: opacity.value + "%" }
        }
        QQC2.CheckBox {
            id: background
            Kirigami.FormData.label: "Background:"
            text: "Show widget background (desktop placement)"
        }
        QQC2.CheckBox {
            id: pinned
            Kirigami.FormData.label: "Popup:"
            text: "Keep open and on top (pin)"
        }
    }
}
