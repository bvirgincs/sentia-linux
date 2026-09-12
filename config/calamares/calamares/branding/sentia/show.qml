import QtQuick 2.0
import calamares.slideshow 1.0

Presentation {
    id: presentation

    Timer {
        interval: 20000
        repeat: true
        onTriggered: presentation.goToNextSlide()
    }

    Slide {
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: parent.top
            anchors.topMargin: 120
            width: parent.width * 0.8
            wrapMode: Text.WordWrap
            horizontalAlignment: Text.AlignHCenter
            color: "#11314b"
            font.pixelSize: 34
            text: qsTr("Sentia Linux installs Debian 13 Trixie with a minimal desktop and offline baseline packages.")
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: parent.top
            anchors.topMargin: 250
            width: parent.width * 0.78
            wrapMode: Text.WordWrap
            horizontalAlignment: Text.AlignHCenter
            color: "#1d4d77"
            font.pixelSize: 24
            text: qsTr("The installer copies the live filesystem directly, including packaged local-model assets when present.")
        }
    }
}
