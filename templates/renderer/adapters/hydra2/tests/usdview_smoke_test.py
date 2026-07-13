# SPDX-License-Identifier: Apache-2.0
import os

from pxr import Gf, Vt


def _frames():
    path = os.environ["{{NAME}}_HYDRA_EVIDENCE"]
    if not os.path.exists(path):
        return []
    result = []
    with open(path, encoding="utf-8") as stream:
        for line in stream:
            fields = {}
            for field in line.split():
                key, value = field.split("=", 1)
                fields[key] = int(value)
            result.append(fields)
    return result


def _render(app_controller, phase):
    before = len(_frames())
    app_controller._stageView.SetForceRefresh(True)
    app_controller._stageView.updateView()
    image_root = os.environ["{{NAME}}_HYDRA_IMAGE"]
    app_controller._takeShot(
        f"{image_root}-{phase}.png", iterations=10, waitForConvergence=True)
    frames = _frames()
    assert len(frames) > before, f"no Hydra frame completed for {phase}"
    assert frames[-1]["buffers_written"] >= 2
    assert frames[-1]["width"] > 0 and frames[-1]["height"] > 0
    return frames[-1]


def testUsdviewInputFunction(appController):
    appController._dataModel.viewSettings.showBBoxes = False
    appController._dataModel.viewSettings.showHUD = False

    first = _render(appController, "first-frame")
    appController._dataModel.stage.GetPrimAtPath(
        "/World/Triangle").GetAttribute("points").Set(
            Vt.Vec3fArray([
                Gf.Vec3f(-0.4, -0.45, 0),
                Gf.Vec3f(0.4, -0.45, 0),
                Gf.Vec3f(0, 0.6, 0),
            ]))
    updated = _render(appController, "stable-update")
    assert updated["frame"] > first["frame"]
    assert updated["scene_revision"] > first["scene_revision"]
