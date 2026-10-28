import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.components as PC
import org.kde.plasma.plasma5support as P5Support
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    readonly property string command: "$HOME/.local/bin/ai-usage --json"
    readonly property int refreshMs: 3 * 60 * 1000
    property var providers: []
    property string updated: ""
    property string fatal: ""

    function colorFor(p) {
        if (p >= 85) return Kirigami.Theme.negativeTextColor
        if (p >= 60) return Kirigami.Theme.neutralTextColor
        return Kirigami.Theme.positiveTextColor
    }
    function maxPercent(prov) {
        let m = 0
        for (const w of prov.windows) m = Math.max(m, w.percent)
        return m
    }
    function shortName(name) { return name === "Claude" ? "C" : "OC" }
    function refresh() { exec.connectSource(command) }

    P5Support.DataSource {
        id: exec
        engine: "executable"
        onNewData: (source, data) => {
            disconnectSource(source)
            try {
                const j = JSON.parse(data["stdout"])
                root.providers = j.providers
                root.updated = new Date().toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
                root.fatal = ""
            } catch (e) {
                root.fatal = data["stderr"] || "ai-usage failed (is it in ~/.local/bin?)"
            }
        }
    }

    Timer {
        interval: root.refreshMs
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    toolTipMainText: "AI Usage"
    toolTipSubText: {
        if (fatal) return fatal
        let lines = []
        for (const p of providers) {
            if (p.error) { lines.push(p.name + ": " + p.error); continue }
            lines.push(p.name + ": " + p.windows.map(w => w.label + " " + Math.round(w.percent) + "%").join(" · "))
        }
        return lines.join("\n")
    }

    component Bar: Rectangle {
        property real percent: 0
        radius: height / 2
        color: Qt.alpha(Kirigami.Theme.textColor, 0.15)
        Rectangle {
            width: parent.width * Math.min(1, parent.percent / 100)
            height: parent.height
            radius: parent.radius
            color: root.colorFor(parent.percent)
        }
    }

    compactRepresentation: MouseArea {
        Layout.minimumWidth: row.implicitWidth
        Layout.preferredWidth: row.implicitWidth
        onClicked: root.expanded = !root.expanded

        RowLayout {
            id: row
            anchors.fill: parent
            spacing: Kirigami.Units.largeSpacing

            PC.Label {
                visible: root.fatal !== "" || root.providers.length === 0
                text: root.fatal ? "AI ⚠" : "AI …"
            }

            Repeater {
                model: root.providers
                RowLayout {
                    required property var modelData
                    spacing: Kirigami.Units.smallSpacing
                    PC.Label {
                        text: root.shortName(modelData.name)
                        font.bold: true
                        opacity: 0.8
                    }
                    ColumnLayout {
                        spacing: 2
                        Layout.alignment: Qt.AlignVCenter
                        Repeater {
                            model: modelData.windows
                            Bar {
                                required property var modelData
                                percent: modelData.percent
                                Layout.preferredWidth: Kirigami.Units.gridUnit * 2.5
                                Layout.preferredHeight: 3
                            }
                        }
                    }
                    PC.Label {
                        text: modelData.error ? "!" : Math.round(root.maxPercent(modelData)) + "%"
                        color: modelData.error ? Kirigami.Theme.negativeTextColor
                                               : root.colorFor(root.maxPercent(modelData))
                    }
                }
            }
        }
    }

    fullRepresentation: ColumnLayout {
        Layout.preferredWidth: Kirigami.Units.gridUnit * 20
        spacing: Kirigami.Units.largeSpacing

        PC.Label {
            visible: root.fatal !== ""
            text: root.fatal
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }

        Repeater {
            model: root.providers
            ColumnLayout {
                required property var modelData
                Layout.fillWidth: true
                Kirigami.Heading { level: 4; text: modelData.name }
                PC.Label {
                    visible: !!modelData.error
                    text: modelData.error || ""
                    color: Kirigami.Theme.negativeTextColor
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
                Repeater {
                    model: modelData.windows
                    RowLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.largeSpacing
                        PC.Label {
                            text: modelData.label
                            Layout.preferredWidth: Kirigami.Units.gridUnit * 3
                        }
                        Bar {
                            percent: modelData.percent
                            Layout.fillWidth: true
                            Layout.preferredHeight: 6
                        }
                        PC.Label {
                            text: Math.round(modelData.percent) + "%"
                            color: root.colorFor(modelData.percent)
                            horizontalAlignment: Text.AlignRight
                            Layout.preferredWidth: Kirigami.Units.gridUnit * 2
                        }
                        PC.Label {
                            text: modelData.resets_in ? "↻ " + modelData.resets_in : ""
                            opacity: 0.6
                            Layout.preferredWidth: Kirigami.Units.gridUnit * 4
                        }
                    }
                }
            }
        }

        Item { Layout.fillHeight: true }

        RowLayout {
            Layout.fillWidth: true
            PC.Label {
                text: root.updated ? "Updated " + root.updated : ""
                opacity: 0.6
                Layout.fillWidth: true
            }
            PC.ToolButton {
                icon.name: "view-refresh"
                onClicked: root.refresh()
            }
        }
    }
}
