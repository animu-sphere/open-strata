"""usdview host add-on scaffolded by OpenStrata."""

from pxr import Tf
from pxr.Usdviewq.plugin import PluginContainer


class {{Name}}PluginContainer(PluginContainer):
    """Minimal host add-on; register project commands in these hooks."""

    def registerPlugins(self, plugRegistry, plugCtx):
        del plugRegistry, plugCtx

    def configureView(self, plugRegistry, plugUIBuilder):
        del plugRegistry, plugUIBuilder


Tf.Type.Define({{Name}}PluginContainer)
