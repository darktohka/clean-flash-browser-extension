package
{
    import flash.display.Sprite;
    import flash.display.Shape;
    import flash.text.TextField;
    import flash.text.TextFieldAutoSize;
    import flash.text.TextFormat;
    import flash.events.Event;
    import flash.events.IOErrorEvent;
    import flash.events.SecurityErrorEvent;
    import flash.events.HTTPStatusEvent;
    import flash.net.URLLoader;
    import flash.net.URLRequest;
    import flash.net.URLRequestMethod;
    import flash.utils.Timer;
    import flash.events.TimerEvent;
    import flash.events.MouseEvent;

    /**
     * URL Rewrite Test Suite
     *
     * Tests that the data-url-rewriter JS callback and extension rewrite
     * rules correctly rewrite URLs before Flash makes HTTP requests.
     *
     * The host HTML page defines a JS function that rewrites specific URLs:
     *   - /rewrite-src/text  →  /text        (simple path rewrite)
     *   - /rewrite-src/json  →  /json        (another path rewrite)
     *   - /rewrite-src/echo  →  /echo        (method/body passthrough)
     *   - /rewrite-src/gone  →  /text        (404 path → valid path)
     *   - /no-rewrite/text   →  (unchanged)  (non-matching URL)
     *
     * Tests verify that the data received matches the REWRITTEN target,
     * not the original URL (which would 404 if not rewritten).
     */
    [SWF(width="800", height="600", backgroundColor="#1e1e2e", frameRate="30")]
    public class URLRewriteTests extends Sprite
    {
        private static const SERVER:String = "http://localhost:3000";

        // Test states
        private static const PENDING:int  = 0;
        private static const RUNNING:int  = 1;
        private static const PASS:int     = 2;
        private static const FAIL:int     = 3;

        // Colors
        private static const COLOR_BG:uint         = 0x1e1e2e;
        private static const COLOR_HEADER_BG:uint   = 0x181825;
        private static const COLOR_TABLE_BG:uint    = 0x11111b;
        private static const COLOR_LOG_BG:uint      = 0x11111b;
        private static const COLOR_PENDING:uint     = 0x585b70;
        private static const COLOR_RUNNING:uint     = 0x89b4fa;
        private static const COLOR_PASS:uint        = 0xa6e3a1;
        private static const COLOR_FAIL:uint        = 0xf38ba8;
        private static const COLOR_TEXT:uint         = 0xcdd6f4;
        private static const COLOR_DIM:uint          = 0x6c7086;
        private static const COLOR_ROW_EVEN:uint     = 0x181825;
        private static const COLOR_ROW_ODD:uint      = 0x1e1e2e;

        // Layout
        private static const STAGE_W:int = 800;
        private static const STAGE_H:int = 600;
        private static const HEADER_H:int = 36;
        private static const TABLE_H:int = 300;
        private static const ROW_H:int = 22;
        private static const COL_NUM:int = 36;
        private static const COL_NAME:int = 380;
        private static const COL_STATUS:int = 80;

        private var tests:Array = [];
        private var currentTestIndex:int = -1;
        private var testTimer:Timer;

        private var headerField:TextField;
        private var tableContainer:Sprite;
        private var tableContent:Sprite;
        private var logField:TextField;
        private var tableScrollY:Number = 0;
        private var tableMask:Shape;

        private var passed:int = 0;
        private var failed:int = 0;
        private var rowFields:Array = [];

        public function URLRewriteTests()
        {
            addEventListener(Event.ADDED_TO_STAGE, onAddedToStage);
        }

        private function onAddedToStage(e:Event):void
        {
            removeEventListener(Event.ADDED_TO_STAGE, onAddedToStage);
            buildUI();
            registerTests();
            renderTable();
            updateHeader();

            log("URL Rewrite Test Suite initialized. " + tests.length + " tests.");
            log("Server: " + SERVER);
            log("Starting tests in 0.5s...");

            var startDelay:Timer = new Timer(500, 1);
            startDelay.addEventListener(TimerEvent.TIMER, function(te:TimerEvent):void {
                runNextTest();
            });
            startDelay.start();
        }

        // ─────────────────── UI ───────────────────

        private function buildUI():void
        {
            var bg:Shape = new Shape();
            bg.graphics.beginFill(COLOR_BG);
            bg.graphics.drawRect(0, 0, STAGE_W, STAGE_H);
            bg.graphics.endFill();
            addChild(bg);

            var headerBg:Shape = new Shape();
            headerBg.graphics.beginFill(COLOR_HEADER_BG);
            headerBg.graphics.drawRect(0, 0, STAGE_W, HEADER_H);
            headerBg.graphics.endFill();
            addChild(headerBg);

            headerField = makeText("URL Rewrite Tests - Initializing...", 10, 8, STAGE_W - 20, HEADER_H, 15, true, COLOR_TEXT);
            addChild(headerField);

            var colHeaderY:int = HEADER_H;
            var colBg:Shape = new Shape();
            colBg.graphics.beginFill(0x313244);
            colBg.graphics.drawRect(0, colHeaderY, STAGE_W, ROW_H);
            colBg.graphics.endFill();
            addChild(colBg);

            var colX:int = 0;
            addChild(makeText("#", colX + 4, colHeaderY + 2, COL_NUM, ROW_H, 11, true, COLOR_TEXT));
            colX += COL_NUM;
            addChild(makeText("Test Name", colX + 4, colHeaderY + 2, COL_NAME, ROW_H, 11, true, COLOR_TEXT));
            colX += COL_NAME;
            addChild(makeText("Status", colX + 4, colHeaderY + 2, COL_STATUS, ROW_H, 11, true, COLOR_TEXT));
            colX += COL_STATUS;
            addChild(makeText("Detail", colX + 4, colHeaderY + 2, STAGE_W - colX, ROW_H, 11, true, COLOR_TEXT));

            var tableY:int = HEADER_H + ROW_H;
            var tableBg:Shape = new Shape();
            tableBg.graphics.beginFill(COLOR_TABLE_BG);
            tableBg.graphics.drawRect(0, tableY, STAGE_W, TABLE_H);
            tableBg.graphics.endFill();
            addChild(tableBg);

            tableContainer = new Sprite();
            tableContainer.y = tableY;
            addChild(tableContainer);

            tableMask = new Shape();
            tableMask.graphics.beginFill(0);
            tableMask.graphics.drawRect(0, tableY, STAGE_W, TABLE_H);
            tableMask.graphics.endFill();
            addChild(tableMask);
            tableContainer.mask = tableMask;

            tableContent = new Sprite();
            tableContainer.addChild(tableContent);

            var logLabelY:int = tableY + TABLE_H;
            var logLabelBg:Shape = new Shape();
            logLabelBg.graphics.beginFill(0x313244);
            logLabelBg.graphics.drawRect(0, logLabelY, STAGE_W, 20);
            logLabelBg.graphics.endFill();
            addChild(logLabelBg);
            addChild(makeText("Trace Log", 6, logLabelY + 2, 200, 18, 11, true, COLOR_TEXT));

            var logY:int = logLabelY + 20;
            var logBg:Shape = new Shape();
            logBg.graphics.beginFill(COLOR_LOG_BG);
            logBg.graphics.drawRect(0, logY, STAGE_W, STAGE_H - logY);
            logBg.graphics.endFill();
            addChild(logBg);

            logField = new TextField();
            logField.x = 6;
            logField.y = logY + 2;
            logField.width = STAGE_W - 12;
            logField.height = STAGE_H - logY - 4;
            logField.multiline = true;
            logField.wordWrap = true;
            logField.selectable = true;
            logField.defaultTextFormat = new TextFormat("_typewriter", 11, COLOR_DIM);
            logField.text = "";
            addChild(logField);

            stage.addEventListener(MouseEvent.MOUSE_WHEEL, onMouseWheel);
        }

        private function onMouseWheel(e:MouseEvent):void
        {
            if (e.stageY < HEADER_H + ROW_H || e.stageY > HEADER_H + ROW_H + TABLE_H) return;
            var maxScroll:Number = Math.max(0, tests.length * ROW_H - TABLE_H);
            tableScrollY = Math.max(0, Math.min(maxScroll, tableScrollY - e.delta * 12));
            tableContent.y = -tableScrollY;
        }

        private function makeText(text:String, x:Number, y:Number, w:Number, h:Number,
                                  size:int = 12, bold:Boolean = false, color:uint = 0xFFFFFF):TextField
        {
            var tf:TextField = new TextField();
            tf.x = x; tf.y = y;
            tf.width = w; tf.height = h;
            tf.selectable = false;
            tf.defaultTextFormat = new TextFormat("_typewriter", size, color, bold);
            tf.text = text;
            return tf;
        }

        // ─────────────────── TABLE ───────────────────

        private function renderTable():void
        {
            while (tableContent.numChildren > 0) tableContent.removeChildAt(0);
            rowFields = [];

            for (var i:int = 0; i < tests.length; i++)
            {
                var t:Object = tests[i];
                var ry:int = i * ROW_H;
                var bgColor:uint = (i % 2 == 0) ? COLOR_ROW_EVEN : COLOR_ROW_ODD;

                var rowBg:Shape = new Shape();
                rowBg.graphics.beginFill(bgColor);
                rowBg.graphics.drawRect(0, ry, STAGE_W, ROW_H);
                rowBg.graphics.endFill();
                tableContent.addChild(rowBg);

                var statusColor:uint = getStatusColor(t.status);
                var statusText:String = getStatusText(t.status);

                var colX:int = 0;
                tableContent.addChild(makeText(String(i + 1), colX + 4, ry + 2, COL_NUM, ROW_H, 11, false, COLOR_DIM));
                colX += COL_NUM;
                tableContent.addChild(makeText(t.name, colX + 4, ry + 2, COL_NAME, ROW_H, 11, false, COLOR_TEXT));
                colX += COL_NAME;

                var statF:TextField = makeText(statusText, colX + 4, ry + 2, COL_STATUS, ROW_H, 11, true, statusColor);
                tableContent.addChild(statF);
                colX += COL_STATUS;

                var detF:TextField = makeText(t.detail || "", colX + 4, ry + 2, STAGE_W - colX - 8, ROW_H, 11, false, COLOR_DIM);
                tableContent.addChild(detF);

                rowFields.push({ bg: rowBg, stat: statF, detail: detF, y: ry });
            }
        }

        private function updateRow(index:int):void
        {
            var t:Object = tests[index];
            var rf:Object = rowFields[index];
            var statusColor:uint = getStatusColor(t.status);
            rf.stat.text = getStatusText(t.status);
            rf.stat.setTextFormat(new TextFormat("_typewriter", 11, statusColor, true));
            rf.detail.text = t.detail || "";
            rf.detail.setTextFormat(new TextFormat("_typewriter", 11, COLOR_DIM));

            var bgColor:uint;
            if (t.status == RUNNING) bgColor = 0x1e2030;
            else bgColor = (index % 2 == 0) ? COLOR_ROW_EVEN : COLOR_ROW_ODD;
            rf.bg.graphics.clear();
            rf.bg.graphics.beginFill(bgColor);
            rf.bg.graphics.drawRect(0, rf.y, STAGE_W, ROW_H);
            rf.bg.graphics.endFill();
        }

        private function getStatusColor(status:int):uint
        {
            switch(status)
            {
                case PENDING: return COLOR_PENDING;
                case RUNNING: return COLOR_RUNNING;
                case PASS:    return COLOR_PASS;
                case FAIL:    return COLOR_FAIL;
                default:      return COLOR_TEXT;
            }
        }

        private function getStatusText(status:int):String
        {
            switch(status)
            {
                case PENDING: return "PENDING";
                case RUNNING: return "RUNNING";
                case PASS:    return "PASS";
                case FAIL:    return "FAIL";
                default:      return "?";
            }
        }

        private function updateHeader():void
        {
            var total:int = tests.length;
            var ran:int = passed + failed;
            headerField.text = "URL Rewrite Tests — " + ran + "/" + total
                + "  |  ✓ " + passed + "  ✗ " + failed;
            headerField.setTextFormat(new TextFormat("_typewriter", 15, COLOR_TEXT, true));
        }

        private function log(msg:String):void
        {
            logField.appendText(msg + "\n");
            logField.scrollV = logField.maxScrollV;
            trace(msg);
        }

        // ─────────────────── TEST RUNNER ───────────────────

        private function addTest(name:String, fn:Function):void
        {
            tests.push({ name: name, fn: fn, status: PENDING, detail: "" });
        }

        private function runNextTest():void
        {
            currentTestIndex++;
            if (currentTestIndex >= tests.length)
            {
                log("");
                log("=== ALL TESTS COMPLETE ===");
                log("Passed: " + passed + "  Failed: " + failed + "  Total: " + tests.length);
                updateHeader();
                return;
            }

            var t:Object = tests[currentTestIndex];
            t.status = RUNNING;
            updateRow(currentTestIndex);
            log("[" + (currentTestIndex + 1) + "/" + tests.length + "] " + t.name);

            var idx:int = currentTestIndex;

            // Timeout safety net
            testTimer = new Timer(10000, 1);
            testTimer.addEventListener(TimerEvent.TIMER, function(te:TimerEvent):void {
                if (tests[idx].status == RUNNING) {
                    markFail(idx, "TIMEOUT (10s)");
                }
            });
            testTimer.start();

            try {
                t.fn(
                    function(detail:String):void { markPass(idx, detail); },
                    function(detail:String):void { markFail(idx, detail); }
                );
            } catch (err:Error) {
                markFail(idx, "Exception: " + err.message);
            }
        }

        private function markPass(idx:int, detail:String):void
        {
            if (tests[idx].status != RUNNING) return;
            tests[idx].status = PASS;
            tests[idx].detail = detail;
            passed++;
            if (testTimer) { testTimer.stop(); testTimer = null; }
            updateRow(idx);
            updateHeader();
            log("  ✓ PASS: " + detail);
            runNextTest();
        }

        private function markFail(idx:int, detail:String):void
        {
            if (tests[idx].status != RUNNING) return;
            tests[idx].status = FAIL;
            tests[idx].detail = detail;
            failed++;
            if (testTimer) { testTimer.stop(); testTimer = null; }
            updateRow(idx);
            updateHeader();
            log("  ✗ FAIL: " + detail);
            runNextTest();
        }

        // ─────────────────── TEST REGISTRATION ───────────────────

        private function registerTests():void
        {
            // -- JS callback rewrite tests (data-url-rewriter) --
            // The HTML page defines a rewriter that maps:
            //   /rewrite-src/text  →  /text
            //   /rewrite-src/json  →  /json
            //   /rewrite-src/echo  →  /echo
            //   /rewrite-src/gone  →  /text  (rescues a 404)
            // URLs not matching /rewrite-src/ are left unchanged.

            addTest("Rewrite: simple text endpoint", testRewriteText);
            addTest("Rewrite: JSON endpoint", testRewriteJson);
            addTest("Rewrite: POST body preserved after rewrite", testRewritePost);
            addTest("Rewrite: 404 path rescued by rewrite", testRewriteRescue404);
            addTest("No-rewrite: non-matching URL unchanged", testNoRewritePassthrough);
            addTest("No-rewrite: direct /text still works", testDirectTextStillWorks);

            // -- Extension regex rule tests --
            // The HTML page also configures extension settings with rules:
            //   ^(.*)/rr-source/data$  →  $1/text
            //   ^(.*)/rr-chain-a/(.*)$ →  $1/rr-chain-b/$2
            //   ^(.*)/rr-chain-b/(.*)$ →  $1/$2
            // These test cascading and backreference support.

            addTest("Regex rule: simple backreference rewrite", testRegexSimple);
            addTest("Regex rule: cascading rules (A→B→final)", testRegexCascade);
        }

        // ─────────────────── TESTS: JS CALLBACK REWRITES ───────────────────

        /**
         * Request /rewrite-src/text which the JS rewriter maps to /text.
         * If rewriting works, we get "Hello, World!".
         * If NOT rewritten, the server returns 404.
         */
        private function testRewriteText(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = new URLLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Received: " + data.substr(0, 60));
                if (data == "Hello, World!") onPass("Rewritten to /text → got expected data");
                else onFail("Unexpected data: " + data);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError (URL probably not rewritten): " + e.text);
            });
            loader.load(new URLRequest(SERVER + "/rewrite-src/text"));
        }

        /**
         * Request /rewrite-src/json → rewritten to /json.
         * Verify we get a JSON response with "status":"ok".
         */
        private function testRewriteJson(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = new URLLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Received: " + data.substr(0, 80));
                if (data.indexOf('"status"') >= 0 && data.indexOf('"ok"') >= 0)
                    onPass("Rewritten to /json → got JSON with status:ok");
                else
                    onFail("Data does not look like /json response: " + data);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER + "/rewrite-src/json"));
        }

        /**
         * POST to /rewrite-src/echo → rewritten to /echo.
         * Verify the POST body is preserved through the rewrite.
         */
        private function testRewritePost(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = new URLLoader();
            var req:URLRequest = new URLRequest(SERVER + "/rewrite-src/echo");
            req.method = URLRequestMethod.POST;
            req.data = "hello=world";
            req.contentType = "application/x-www-form-urlencoded";

            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Echo response: " + data.substr(0, 120));
                if (data.indexOf("hello=world") >= 0)
                    onPass("POST body preserved through rewrite");
                else
                    onFail("POST body not found in echo: " + data.substr(0, 100));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(req);
        }

        /**
         * Request /rewrite-src/gone → rewritten to /text.
         * Without rewriting, /rewrite-src/gone would 404.
         * This verifies the rewrite rescues the request.
         */
        private function testRewriteRescue404(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = new URLLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                if (data == "Hello, World!")
                    onPass("404 path rewritten to valid /text endpoint");
                else
                    onFail("Unexpected data: " + data);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError (rewrite did not rescue 404): " + e.text);
            });
            loader.load(new URLRequest(SERVER + "/rewrite-src/gone"));
        }

        /**
         * Request /no-rewrite/text — the rewriter should NOT match this.
         * The server returns 404 for /no-rewrite/text, proving no rewrite happened.
         */
        private function testNoRewritePassthrough(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = new URLLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                // If we get here, the server somehow returned 200 for /no-rewrite/text.
                // That means either the server has this route or something unexpected.
                onFail("Expected 404 but got 200 — was the URL rewritten?");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                log("  Got expected IOError for non-rewritten path");
                onPass("Non-matching URL correctly not rewritten (404)");
            });
            loader.load(new URLRequest(SERVER + "/no-rewrite/text"));
        }

        /**
         * Direct request to /text (no rewrite needed) still works fine.
         */
        private function testDirectTextStillWorks(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = new URLLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                if (data == "Hello, World!") onPass("Direct /text works normally");
                else onFail("Unexpected data: " + data);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER + "/text"));
        }

        // ─────────────────── TESTS: EXTENSION REGEX RULES ───────────────────

        /**
         * Request /rr-source/data → regex rule rewrites to /text.
         * Uses backreference: ^(.*)/rr-source/data$ → $1/text
         */
        private function testRegexSimple(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = new URLLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Received: " + data.substr(0, 60));
                if (data == "Hello, World!")
                    onPass("Regex backreference rewrote /rr-source/data → /text");
                else
                    onFail("Unexpected data: " + data);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError (regex rewrite failed): " + e.text);
            });
            loader.load(new URLRequest(SERVER + "/rr-source/data"));
        }

        /**
         * Request /rr-chain-a/text → cascading rules:
         *   Rule 1: /rr-chain-a/text → /rr-chain-b/text
         *   Rule 2: /rr-chain-b/text → /text
         * Final result: /text → "Hello, World!"
         */
        private function testRegexCascade(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = new URLLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Received: " + data.substr(0, 60));
                if (data == "Hello, World!")
                    onPass("Cascade: /rr-chain-a/text → /rr-chain-b/text → /text");
                else
                    onFail("Unexpected data (cascade may have failed): " + data);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError (cascade rewrite failed): " + e.text);
            });
            loader.load(new URLRequest(SERVER + "/rr-chain-a/text"));
        }
    }
}
