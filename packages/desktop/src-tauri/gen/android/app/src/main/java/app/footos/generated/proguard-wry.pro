# THIS FILE IS AUTO-GENERATED. DO NOT MODIFY!!

# Copyright 2020-2023 Tauri Programme within The Commons Conservancy
# SPDX-License-Identifier: Apache-2.0
# SPDX-License-Identifier: MIT

-keep class app.footos.* {
  native <methods>;
}

-keep class app.footos.WryActivity {
  public <init>(...);

  void setWebView(app.footos.RustWebView);
  java.lang.Class getAppClass(...);
  java.lang.String getVersion();
}

-keep class app.footos.Ipc {
  public <init>(...);

  @android.webkit.JavascriptInterface public <methods>;
}

-keep class app.footos.RustWebView {
  public <init>(...);

  void loadUrlMainThread(...);
  void loadHTMLMainThread(...);
  void evalScript(...);
}

-keep class app.footos.RustWebChromeClient,app.footos.RustWebViewClient {
  public <init>(...);
}
