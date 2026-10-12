// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <core/engine/inputplugin.h>
#include <core/plugins/plugin.h>

#include <QObject>

class TtaPlugin final : public QObject, public Fooyin::Plugin, public Fooyin::InputPlugin
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID "org.fooyin.fooyin.plugin/1.0" FILE "tta.json")
    Q_INTERFACES(Fooyin::Plugin Fooyin::InputPlugin)

  public:
    [[nodiscard]] QString inputName() const override;
    [[nodiscard]] Fooyin::InputCreator inputCreator() const override;
};
