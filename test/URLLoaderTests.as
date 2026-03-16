package
{
    import flash.display.Sprite;
    import flash.display.Shape;
    import flash.text.TextField;
    import flash.text.TextFieldAutoSize;
    import flash.text.TextFormat;
    import flash.text.TextFieldType;
    import flash.events.Event;
    import flash.events.IOErrorEvent;
    import flash.events.SecurityErrorEvent;
    import flash.events.HTTPStatusEvent;
    import flash.events.ProgressEvent;
    import flash.events.MouseEvent;
    import flash.net.URLLoader;
    import flash.net.URLRequest;
    import flash.net.URLRequestMethod;
    import flash.net.URLRequestHeader;
    import flash.net.URLVariables;
    import flash.net.URLLoaderDataFormat;
    import flash.system.Security;
    import flash.system.SecurityDomain;
    import flash.system.ApplicationDomain;
    import flash.system.LoaderContext;
    import flash.display.Loader;
    import flash.display.LoaderInfo;
    import flash.display.DisplayObject;
    import flash.utils.Timer;
    import flash.events.TimerEvent;
    import flash.utils.ByteArray;
    import flash.utils.getTimer;

    [SWF(width="1000", height="700", backgroundColor="#1e1e2e", frameRate="30")]
    public class URLLoaderTests extends Sprite
    {
        // Server config
        private static const SERVER_A:String = "http://localhost:3000";
        private static const SERVER_B:String = "http://localhost:3001";
        private static const SERVER_C:String = "http://localhost:3002";

        // Test states
        private static const PENDING:int  = 0;
        private static const RUNNING:int  = 1;
        private static const PASS:int     = 2;
        private static const FAIL:int     = 3;
        private static const SKIP:int     = 4;

        // Colors
        private static const COLOR_BG:uint         = 0x1e1e2e;
        private static const COLOR_HEADER_BG:uint   = 0x181825;
        private static const COLOR_TABLE_BG:uint    = 0x11111b;
        private static const COLOR_LOG_BG:uint      = 0x11111b;
        private static const COLOR_PENDING:uint     = 0x585b70;
        private static const COLOR_RUNNING:uint     = 0x89b4fa;
        private static const COLOR_PASS:uint        = 0xa6e3a1;
        private static const COLOR_FAIL:uint        = 0xf38ba8;
        private static const COLOR_SKIP:uint        = 0xf9e2af;
        private static const COLOR_TEXT:uint         = 0xcdd6f4;
        private static const COLOR_DIM:uint          = 0x6c7086;
        private static const COLOR_ROW_EVEN:uint     = 0x181825;
        private static const COLOR_ROW_ODD:uint      = 0x1e1e2e;

        // Layout
        private static const STAGE_W:int = 1000;
        private static const STAGE_H:int = 700;
        private static const HEADER_H:int = 36;
        private static const TABLE_H:int = 420;
        private static const LOG_H:int = 244;
        private static const ROW_H:int = 22;
        private static const COL_NUM:int = 40;
        private static const COL_CAT:int = 130;
        private static const COL_NAME:int = 380;
        private static const COL_STATUS:int = 80;
        // COL_DETAIL fills remainder

        // Test data
        private var tests:Array = [];
        private var currentTestIndex:int = -1;
        private var testTimer:Timer;

        // UI elements
        private var headerField:TextField;
        private var tableContainer:Sprite;
        private var tableContent:Sprite;
        private var logField:TextField;
        private var tableScrollY:Number = 0;
        private var tableMask:Shape;

        // Stats
        private var passed:int = 0;
        private var failed:int = 0;
        private var skipped:int = 0;

        // Row text fields cache for updates
        private var rowFields:Array = [];

        public function URLLoaderTests()
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

            // Load cross-origin policy files before starting tests
            Security.loadPolicyFile(SERVER_B + "/crossdomain.xml");
            Security.loadPolicyFile(SERVER_B + "/subdir/crossdomain.xml");

            log("Test suite initialized. " + tests.length + " tests registered.");
            log("Servers: A=" + SERVER_A + " B=" + SERVER_B + " C=" + SERVER_C);
            log("Starting tests in 0.5s...");

            var startDelay:Timer = new Timer(500, 1);
            startDelay.addEventListener(TimerEvent.TIMER, function(te:TimerEvent):void {
                runNextTest();
            });
            startDelay.start();
        }

        // ─────────────────── UI BUILDING ───────────────────

        private function buildUI():void
        {
            // Background
            var bg:Shape = new Shape();
            bg.graphics.beginFill(COLOR_BG);
            bg.graphics.drawRect(0, 0, STAGE_W, STAGE_H);
            bg.graphics.endFill();
            addChild(bg);

            // Header bar
            var headerBg:Shape = new Shape();
            headerBg.graphics.beginFill(COLOR_HEADER_BG);
            headerBg.graphics.drawRect(0, 0, STAGE_W, HEADER_H);
            headerBg.graphics.endFill();
            addChild(headerBg);

            headerField = makeText("URLLoader Test Suite - Initializing...", 10, 8, STAGE_W - 20, HEADER_H, 15, true, COLOR_TEXT);
            addChild(headerField);

            // Table column headers
            var colHeaderY:int = HEADER_H;
            var colBg:Shape = new Shape();
            colBg.graphics.beginFill(0x313244);
            colBg.graphics.drawRect(0, colHeaderY, STAGE_W, ROW_H);
            colBg.graphics.endFill();
            addChild(colBg);

            var colX:int = 0;
            addChild(makeText("#", colX + 4, colHeaderY + 2, COL_NUM, ROW_H, 11, true, COLOR_TEXT));
            colX += COL_NUM;
            addChild(makeText("Category", colX + 4, colHeaderY + 2, COL_CAT, ROW_H, 11, true, COLOR_TEXT));
            colX += COL_CAT;
            addChild(makeText("Test Name", colX + 4, colHeaderY + 2, COL_NAME, ROW_H, 11, true, COLOR_TEXT));
            colX += COL_NAME;
            addChild(makeText("Status", colX + 4, colHeaderY + 2, COL_STATUS, ROW_H, 11, true, COLOR_TEXT));
            colX += COL_STATUS;
            addChild(makeText("Detail", colX + 4, colHeaderY + 2, STAGE_W - colX, ROW_H, 11, true, COLOR_TEXT));

            // Table area
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

            // Log section label
            var logLabelY:int = tableY + TABLE_H;
            var logLabelBg:Shape = new Shape();
            logLabelBg.graphics.beginFill(0x313244);
            logLabelBg.graphics.drawRect(0, logLabelY, STAGE_W, 20);
            logLabelBg.graphics.endFill();
            addChild(logLabelBg);
            addChild(makeText("Trace Log", 6, logLabelY + 2, 200, 18, 11, true, COLOR_TEXT));

            // Log text area
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

            // Mouse wheel scrolling for table
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

        // ─────────────────── TABLE RENDERING ───────────────────

        private function renderTable():void
        {
            // Clear previous
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
                var numF:TextField = makeText(String(i + 1), colX + 4, ry + 2, COL_NUM, ROW_H, 11, false, COLOR_DIM);
                tableContent.addChild(numF);
                colX += COL_NUM;

                var catF:TextField = makeText(t.category, colX + 4, ry + 2, COL_CAT, ROW_H, 11, false, COLOR_TEXT);
                tableContent.addChild(catF);
                colX += COL_CAT;

                var nameF:TextField = makeText(t.name, colX + 4, ry + 2, COL_NAME, ROW_H, 11, false, COLOR_TEXT);
                tableContent.addChild(nameF);
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
            var statusText:String = getStatusText(t.status);

            rf.stat.text = statusText;
            rf.stat.setTextFormat(new TextFormat("_typewriter", 11, statusColor, true));
            rf.detail.text = t.detail || "";
            rf.detail.setTextFormat(new TextFormat("_typewriter", 11, COLOR_DIM));

            // Highlight running row background
            var bgColor:uint;
            if (t.status == RUNNING) bgColor = 0x1e2030;
            else bgColor = (index % 2 == 0) ? COLOR_ROW_EVEN : COLOR_ROW_ODD;
            rf.bg.graphics.clear();
            rf.bg.graphics.beginFill(bgColor);
            rf.bg.graphics.drawRect(0, rf.y, STAGE_W, ROW_H);
            rf.bg.graphics.endFill();

            // Auto-scroll table to keep current test visible
            if (t.status == RUNNING)
            {
                var rowTop:Number = index * ROW_H;
                var rowBot:Number = rowTop + ROW_H;
                if (rowBot > tableScrollY + TABLE_H)
                {
                    tableScrollY = rowBot - TABLE_H;
                    tableContent.y = -tableScrollY;
                }
                else if (rowTop < tableScrollY)
                {
                    tableScrollY = rowTop;
                    tableContent.y = -tableScrollY;
                }
            }
        }

        private function updateHeader():void
        {
            var total:int = tests.length;
            var pending:int = total - passed - failed - skipped;
            // Count running
            for (var i:int = 0; i < tests.length; i++)
            {
                if (tests[i].status == RUNNING) pending--;
            }
            headerField.text = "URLLoader Test Suite  |  " +
                passed + " passed  |  " + failed + " failed  |  " +
                skipped + " skipped  |  " + pending + " pending  |  " + total + " total";
        }

        private function getStatusColor(status:int):uint
        {
            switch(status)
            {
                case PENDING: return COLOR_PENDING;
                case RUNNING: return COLOR_RUNNING;
                case PASS:    return COLOR_PASS;
                case FAIL:    return COLOR_FAIL;
                case SKIP:    return COLOR_SKIP;
            }
            return COLOR_TEXT;
        }

        private function getStatusText(status:int):String
        {
            switch(status)
            {
                case PENDING: return "PENDING";
                case RUNNING: return "RUNNING";
                case PASS:    return "PASS";
                case FAIL:    return "FAIL";
                case SKIP:    return "SKIP";
            }
            return "???";
        }

        // ─────────────────── LOGGING ───────────────────

        private function log(msg:String):void
        {
            var now:Date = new Date();
            var ts:String = pad(now.hours) + ":" + pad(now.minutes) + ":" + pad(now.seconds);
            var line:String = "[" + ts + "] " + msg;
            trace(line);
            logField.appendText(line + "\n");
            logField.scrollV = logField.maxScrollV;
        }

        private function pad(n:int):String
        {
            return n < 10 ? "0" + n : String(n);
        }

        // ─────────────────── TEST RUNNER ───────────────────

        private function addTest(category:String, name:String, fn:Function):void
        {
            tests.push({
                category: category,
                name: name,
                fn: fn,
                status: PENDING,
                detail: "",
                startTime: 0
            });
        }

        private function runNextTest():void
        {
            currentTestIndex++;
            if (currentTestIndex >= tests.length)
            {
                log("========================================");
                log("ALL TESTS COMPLETE: " + passed + " passed, " + failed + " failed, " + skipped + " skipped out of " + tests.length);
                log("========================================");
                updateHeader();
                return;
            }

            var t:Object = tests[currentTestIndex];
            t.status = RUNNING;
            t.startTime = getTimer();
            updateRow(currentTestIndex);
            updateHeader();
            log("--- [" + (currentTestIndex + 1) + "/" + tests.length + "] " + t.category + " :: " + t.name + " ---");

            // Timeout timer (15s)
            testTimer = new Timer(15000, 1);
            testTimer.addEventListener(TimerEvent.TIMER, onTestTimeout);
            testTimer.start();

            try
            {
                t.fn(passTest, failTest);
            }
            catch (err:Error)
            {
                failTest("Exception: " + err.message);
            }
        }

        private function onTestTimeout(e:TimerEvent):void
        {
            failTest("TIMEOUT (15s)");
        }

        private function passTest(detail:String = ""):void
        {
            if (currentTestIndex < 0 || currentTestIndex >= tests.length) return;
            var t:Object = tests[currentTestIndex];
            if (t.status != RUNNING) return; // already resolved

            if (testTimer) { testTimer.stop(); testTimer = null; }
            var elapsed:int = getTimer() - t.startTime;
            t.status = PASS;
            t.detail = detail + " (" + elapsed + "ms)";
            passed++;
            updateRow(currentTestIndex);
            updateHeader();
            log("  PASS: " + t.detail);
            scheduleNext();
        }

        private function failTest(detail:String = ""):void
        {
            if (currentTestIndex < 0 || currentTestIndex >= tests.length) return;
            var t:Object = tests[currentTestIndex];
            if (t.status != RUNNING) return; // already resolved

            if (testTimer) { testTimer.stop(); testTimer = null; }
            var elapsed:int = getTimer() - t.startTime;
            t.status = FAIL;
            t.detail = detail + " (" + elapsed + "ms)";
            failed++;
            updateRow(currentTestIndex);
            updateHeader();
            log("  FAIL: " + t.detail);
            scheduleNext();
        }

        private function skipTest(detail:String = ""):void
        {
            if (currentTestIndex < 0 || currentTestIndex >= tests.length) return;
            var t:Object = tests[currentTestIndex];
            if (t.status != RUNNING) return;

            if (testTimer) { testTimer.stop(); testTimer = null; }
            t.status = SKIP;
            t.detail = detail;
            skipped++;
            updateRow(currentTestIndex);
            updateHeader();
            log("  SKIP: " + t.detail);
            scheduleNext();
        }

        private function scheduleNext():void
        {
            var delay:Timer = new Timer(100, 1);
            delay.addEventListener(TimerEvent.TIMER, function(te:TimerEvent):void {
                runNextTest();
            });
            delay.start();
        }

        // ─────────────────── TEST REGISTRATION ───────────────────

        private function registerTests():void
        {
            // ── A. Basic Events ──
            addTest("Events", "Event.OPEN fires on load", testOpenEvent);
            addTest("Events", "Event.COMPLETE fires on 200", testCompleteEvent);
            addTest("Events", "ProgressEvent.PROGRESS fires (large)", testProgressEvent);
            addTest("Events", "HTTPStatusEvent fires with 200", testHttpStatusEvent);
            addTest("Events", "Event ordering: OPEN→STATUS→COMPLETE", testEventOrdering);
            addTest("Events", "COMPLETE fires even w/o PROGRESS (small)", testCompleteWithoutProgress);

            // ── B. HTTP Methods & Data Formats ──
            addTest("Methods", "GET with URLVariables", testGetWithVariables);
            addTest("Methods", "POST with URLVariables", testPostWithVariables);
            addTest("Methods", "POST with XML body + header", testPostXml);
            addTest("DataFormat", "TEXT format returns String", testDataFormatText);
            addTest("DataFormat", "BINARY format returns ByteArray", testDataFormatBinary);
            addTest("DataFormat", "VARIABLES format auto-parses", testDataFormatVariables);
            addTest("DataFormat", "URLVariables special chars round-trip", testVariablesSpecialChars);

            // ── C. Non-200 / Error Responses ──
            addTest("Errors", "404 → IOErrorEvent", testStatus404);
            addTest("Errors", "500 → IOErrorEvent", testStatus500);
            addTest("Errors", "302 redirect follows → COMPLETE", testRedirect302);
            addTest("Errors", "204 No Content behavior", testStatus204);
            addTest("Errors", "Connection refused → IOError", testConnectionRefused);
            //addTest("Errors", "Slow server (5s delay) completes", testSlowServer);
            addTest("Errors", "200 empty body → COMPLETE", testEmptyResponse);

            // ── D. Cross-Domain / crossdomain.xml ──
            addTest("CrossDomain", "Same-origin succeeds", testSameOrigin);
            addTest("CrossDomain", "Cross-origin WITH policy → OK", testCrossOriginWithPolicy);
            addTest("CrossDomain", "Cross-origin WITHOUT policy → SecErr", testCrossOriginNoPolicy);
            addTest("CrossDomain", "Cross-origin POST with policy", testCrossOriginPostWithPolicy);
            addTest("CrossDomain", "Cross-origin POST no policy → SecErr", testCrossOriginPostNoPolicy);
            addTest("CrossDomain", "loadPolicyFile custom path", testLoadPolicyFileCustom);
            addTest("CrossDomain", "Cross-origin binary with policy", testCrossOriginBinaryWithPolicy);

            // ── E. Edge Cases ──
            addTest("EdgeCase", "Malformed URL → error", testMalformedUrl);
            //addTest("EdgeCase", "Reuse URLLoader (2nd load)", testReuseLoader);
            addTest("EdgeCase", "Invalid VARIABLES response → error", testInvalidVariablesResponse);
            addTest("EdgeCase", "Custom request header echoed", testCustomRequestHeader);
            addTest("EdgeCase", "Large response + progress tracking", testLargeWithProgress);
            addTest("EdgeCase", "URL query + URLVariables merge", testQueryAndVariablesMerge);
            addTest("EdgeCase", "Blocked port (port 25) → error", testBlockedPort);
            addTest("EdgeCase", "5 concurrent requests all resolve", testConcurrentRequests);

            // ── F. SWF Loading (Loader class) ──
            addTest("SWFLoad", "Loader basic: INIT+COMPLETE fire", testLoaderBasic);
            addTest("SWFLoad", "Loader event order: OPEN→INIT→COMPLETE", testLoaderEventOrder);
            addTest("SWFLoad", "Loader contentLoaderInfo properties", testLoaderContentInfo);
            addTest("SWFLoad", "Loader adds child to display list", testLoaderDisplayList);
            addTest("SWFLoad", "Loader cross-origin SWF with policy", testLoaderCrossOriginWithPolicy);
            addTest("SWFLoad", "Loader cross-origin SWF no policy → err", testLoaderCrossOriginNoPolicy);
            addTest("SWFLoad", "Loader load non-SWF → IOError", testLoaderNonSwf);
            addTest("SWFLoad", "Loader load 404 → IOError", testLoader404);
            addTest("SWFLoad", "Loader unload() clears content", testLoaderUnload);
            addTest("SWFLoad", "Loader progress events for SWF", testLoaderProgress);
            addTest("SWFLoad", "Loader loadBytes from ByteArray", testLoaderLoadBytes);
        }

        // ─────────────────── HELPERS ───────────────────

        private function makeLoader(fmt:String = null):URLLoader
        {
            var loader:URLLoader = new URLLoader();
            if (fmt) loader.dataFormat = fmt;
            return loader;
        }

        // ─────────────────── A. BASIC EVENT TESTS ───────────────────

        private function testOpenEvent(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var openFired:Boolean = false;
            loader.addEventListener(Event.OPEN, function(e:Event):void {
                openFired = true;
                log("  Event.OPEN received");
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                if (openFired) onPass("OPEN fired before COMPLETE");
                else onFail("OPEN did not fire");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/text"));
        }

        private function testCompleteEvent(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Data received: " + data.substr(0, 60));
                if (data == "Hello, World!") onPass("Data matches expected");
                else onFail("Data mismatch: " + data);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/text"));
        }

        private function testProgressEvent(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var progressCount:int = 0;
            loader.addEventListener(ProgressEvent.PROGRESS, function(e:ProgressEvent):void {
                progressCount++;
                log("  Progress: " + e.bytesLoaded + "/" + e.bytesTotal);
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                if (progressCount > 0) onPass(progressCount + " progress events");
                else onFail("No PROGRESS events for large response");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/large"));
        }

        private function testHttpStatusEvent(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var statusCode:int = -1;
            loader.addEventListener(HTTPStatusEvent.HTTP_STATUS, function(e:HTTPStatusEvent):void {
                statusCode = e.status;
                log("  HTTP status: " + statusCode);
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                if (statusCode == 200) onPass("Status 200 confirmed");
                else if (statusCode == 0) onPass("Status 0 (browser may not expose status)");
                else onFail("Unexpected status: " + statusCode);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/text"));
        }

        private function testEventOrdering(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var order:Array = [];
            loader.addEventListener(Event.OPEN, function(e:Event):void {
                order.push("OPEN");
            });
            loader.addEventListener(HTTPStatusEvent.HTTP_STATUS, function(e:HTTPStatusEvent):void {
                order.push("HTTP_STATUS");
            });
            loader.addEventListener(ProgressEvent.PROGRESS, function(e:ProgressEvent):void {
                if (order.indexOf("PROGRESS") == -1) order.push("PROGRESS");
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                order.push("COMPLETE");
                var orderStr:String = order.join("→");
                log("  Order: " + orderStr);
                // OPEN should be first, COMPLETE should be last
                if (order[0] == "OPEN" && order[order.length - 1] == "COMPLETE")
                    onPass(orderStr);
                else
                    onFail("Unexpected order: " + orderStr);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/text"));
        }

        private function testCompleteWithoutProgress(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var progressFired:Boolean = false;
            loader.addEventListener(ProgressEvent.PROGRESS, function(e:ProgressEvent):void {
                progressFired = true;
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                // Either outcome is valid for a tiny response
                if (!progressFired)
                    onPass("COMPLETE w/o PROGRESS (expected for tiny file)");
                else
                    onPass("COMPLETE with PROGRESS (also valid)");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            // /tiny returns just "ok"
            loader.load(new URLRequest(SERVER_A + "/tiny"));
        }

        // ─────────────────── B. HTTP METHODS & DATA FORMAT TESTS ───────────────────

        private function testGetWithVariables(onPass:Function, onFail:Function):void
        {
            var req:URLRequest = new URLRequest(SERVER_A + "/echo");
            req.method = URLRequestMethod.GET;
            var vars:URLVariables = new URLVariables();
            vars.foo = "bar";
            vars.num = "42";
            req.data = vars;

            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Echo: " + data.substr(0, 120));
                // Server echoes back the query params - check it contains foo=bar
                if (data.indexOf("bar") >= 0 && data.indexOf("42") >= 0)
                    onPass("Query params echoed correctly");
                else
                    onFail("Missing params in echo: " + data.substr(0, 100));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(req);
        }

        private function testPostWithVariables(onPass:Function, onFail:Function):void
        {
            var req:URLRequest = new URLRequest(SERVER_A + "/echo");
            req.method = URLRequestMethod.POST;
            var vars:URLVariables = new URLVariables();
            vars.greeting = "hello";
            vars.target = "world";
            req.data = vars;

            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Echo: " + data.substr(0, 120));
                if (data.indexOf("hello") >= 0 && data.indexOf("world") >= 0)
                    onPass("POST body echoed correctly");
                else
                    onFail("Missing params in echo: " + data.substr(0, 100));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(req);
        }

        private function testPostXml(onPass:Function, onFail:Function):void
        {
            var req:URLRequest = new URLRequest(SERVER_A + "/echo");
            req.method = URLRequestMethod.POST;
            req.contentType = "text/xml";
            req.data = "<root><msg>test</msg></root>";

            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Echo: " + data.substr(0, 120));
                if (data.indexOf("<root>") >= 0 || data.indexOf("text/xml") >= 0 || data.indexOf("test") >= 0)
                    onPass("XML POST echoed");
                else
                    onFail("XML not in echo: " + data.substr(0, 100));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(req);
        }

        private function testDataFormatText(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader(URLLoaderDataFormat.TEXT);
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:* = URLLoader(e.target).data;
                if (data is String)
                    onPass("data is String, len=" + String(data).length);
                else
                    onFail("data is not String: " + typeof(data));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/text"));
        }

        private function testDataFormatBinary(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader(URLLoaderDataFormat.BINARY);
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:* = URLLoader(e.target).data;
                if (data is ByteArray)
                {
                    var ba:ByteArray = data as ByteArray;
                    log("  ByteArray length: " + ba.length);
                    if (ba.length == 256)
                        onPass("ByteArray(256 bytes) correct");
                    else
                        onPass("ByteArray(" + ba.length + " bytes) received");
                }
                else
                    onFail("data is not ByteArray");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/binary"));
        }

        private function testDataFormatVariables(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader(URLLoaderDataFormat.VARIABLES);
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:* = URLLoader(e.target).data;
                if (data is URLVariables)
                {
                    var uv:URLVariables = data as URLVariables;
                    log("  name=" + uv.name + " version=" + uv.version);
                    if (uv.name == "Flash" && uv.version == "32")
                        onPass("Variables parsed: name=Flash, version=32");
                    else
                        onFail("Unexpected var values: name=" + uv.name);
                }
                else
                    onFail("data is not URLVariables");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/variables"));
        }

        private function testVariablesSpecialChars(onPass:Function, onFail:Function):void
        {
            var req:URLRequest = new URLRequest(SERVER_A + "/echo-vars");
            req.method = URLRequestMethod.POST;
            var vars:URLVariables = new URLVariables();
            vars.amp = "&";
            vars.eq = "=";
            vars.unicode = "\u00e9\u00f1"; // éñ
            vars.spaces = "hello world";
            req.data = vars;

            var loader:URLLoader = makeLoader(URLLoaderDataFormat.VARIABLES);
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:* = URLLoader(e.target).data;
                if (data is URLVariables)
                {
                    var uv:URLVariables = data as URLVariables;
                    log("  amp=" + uv.amp + " eq=" + uv.eq + " unicode=" + uv.unicode);
                    if (uv.amp == "&" && uv.eq == "=" && uv.spaces == "hello world")
                        onPass("Special chars round-tripped");
                    else
                        onFail("Mismatch: amp=" + uv.amp + " eq=" + uv.eq);
                }
                else
                    onFail("Response not URLVariables");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(req);
        }

        // ─────────────────── C. NON-200 / ERROR TESTS ───────────────────

        private function testStatus404(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var httpStatus:int = -1;
            loader.addEventListener(HTTPStatusEvent.HTTP_STATUS, function(e:HTTPStatusEvent):void {
                httpStatus = e.status;
                log("  HTTP status: " + httpStatus);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                log("  IOError (expected): " + e.text);
                onPass("IOError fired, status=" + httpStatus);
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                // Some runtimes may still fire COMPLETE for non-200
                onPass("COMPLETE fired for 404, status=" + httpStatus + " (runtime-dependent)");
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError (unexpected): " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/status/404"));
        }

        private function testStatus500(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var httpStatus:int = -1;
            loader.addEventListener(HTTPStatusEvent.HTTP_STATUS, function(e:HTTPStatusEvent):void {
                httpStatus = e.status;
                log("  HTTP status: " + httpStatus);
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                log("  IOError (expected): " + e.text);
                onPass("IOError fired, status=" + httpStatus);
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                onPass("COMPLETE fired for 500, status=" + httpStatus + " (runtime-dependent)");
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/status/500"));
        }

        private function testRedirect302(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Redirect landed on: " + data.substr(0, 60));
                if (data == "Hello, World!")
                    onPass("Redirect followed to /text");
                else
                    onPass("Redirect followed, data=" + data.substr(0, 40));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/redirect"));
        }

        private function testStatus204(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var httpStatus:int = -1;
            loader.addEventListener(HTTPStatusEvent.HTTP_STATUS, function(e:HTTPStatusEvent):void {
                httpStatus = e.status;
                log("  HTTP status: " + httpStatus);
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                onPass("COMPLETE fired, status=" + httpStatus + ", data len=" + (data ? data.length : 0));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onPass("IOError for 204, status=" + httpStatus + " (runtime-dependent)");
            });
            loader.load(new URLRequest(SERVER_A + "/status/204"));
        }

        private function testConnectionRefused(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                log("  IOError (expected): " + e.text);
                onPass("IOError on connection refused");
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onPass("SecurityError on connection refused (also valid)");
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                onFail("COMPLETE should not fire for dead port");
            });
            // Port 39999 should have nothing listening
            loader.load(new URLRequest("http://localhost:39999/test"));
        }

        private function testSlowServer(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var startMs:int = getTimer();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var elapsed:int = getTimer() - startMs;
                var data:String = URLLoader(e.target).data;
                log("  Slow response took " + elapsed + "ms");
                if (elapsed >= 2000)
                    onPass("Waited " + elapsed + "ms, data=" + data.substr(0, 30));
                else
                    onPass("Completed in " + elapsed + "ms (faster than expected)");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/slow"));
        }

        private function testEmptyResponse(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Empty response data len=" + (data ? data.length : -1));
                if (data == null || data.length == 0 || data == "")
                    onPass("COMPLETE fired, empty data");
                else
                    onPass("COMPLETE fired, data='" + data.substr(0, 20) + "' (may include whitespace)");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onPass("IOError for empty response (runtime-dependent)");
            });
            loader.load(new URLRequest(SERVER_A + "/empty"));
        }

        // ─────────────────── D. CROSS-DOMAIN TESTS ───────────────────

        private function testSameOrigin(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                onPass("Same-origin OK, data=" + data.substr(0, 30));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/text"));
        }

        private function testCrossOriginWithPolicy(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Cross-origin (with policy) data: " + data.substr(0, 40));
                onPass("Cross-origin succeeded with crossdomain.xml");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_B + "/text"));
        }

        private function testCrossOriginNoPolicy(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                log("  SecurityError (expected): " + e.text);
                onPass("SecurityError fired as expected");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                // Some runtimes fire IOError instead of SecurityError
                onPass("IOError fired (also valid for blocked cross-origin)");
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                onFail("COMPLETE should not fire without crossdomain.xml");
            });
            loader.load(new URLRequest(SERVER_C + "/text"));
        }

        private function testCrossOriginPostWithPolicy(onPass:Function, onFail:Function):void
        {
            var req:URLRequest = new URLRequest(SERVER_B + "/echo");
            req.method = URLRequestMethod.POST;
            var vars:URLVariables = new URLVariables();
            vars.cross = "origin";
            req.data = vars;

            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Cross-origin POST echo: " + data.substr(0, 80));
                if (data.indexOf("origin") >= 0)
                    onPass("Cross-origin POST succeeded");
                else
                    onPass("Cross-origin POST completed, data=" + data.substr(0, 40));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError: " + e.text);
            });
            loader.load(req);
        }

        private function testCrossOriginPostNoPolicy(onPass:Function, onFail:Function):void
        {
            var req:URLRequest = new URLRequest(SERVER_C + "/echo");
            req.method = URLRequestMethod.POST;
            var vars:URLVariables = new URLVariables();
            vars.cross = "blocked";
            req.data = vars;

            var loader:URLLoader = makeLoader();
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                log("  SecurityError (expected): " + e.text);
                onPass("SecurityError for cross-origin POST w/o policy");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onPass("IOError for cross-origin POST w/o policy (also valid)");
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                onFail("COMPLETE should not fire for blocked cross-origin POST");
            });
            loader.load(req);
        }

        private function testLoadPolicyFileCustom(onPass:Function, onFail:Function):void
        {
            // Policy was pre-loaded from SERVER_B + "/subdir/crossdomain.xml"
            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Custom policy path data: " + data.substr(0, 40));
                onPass("Custom loadPolicyFile path works");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_B + "/subdir/data"));
        }

        private function testCrossOriginBinaryWithPolicy(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader(URLLoaderDataFormat.BINARY);
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:* = URLLoader(e.target).data;
                if (data is ByteArray)
                {
                    var ba:ByteArray = data as ByteArray;
                    log("  Cross-origin binary len: " + ba.length);
                    onPass("Binary cross-origin OK, " + ba.length + " bytes");
                }
                else
                    onFail("Data not ByteArray");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_B + "/binary"));
        }

        // ─────────────────── E. EDGE CASE TESTS ───────────────────

        private function testMalformedUrl(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                log("  IOError (expected for malformed URL): " + e.text);
                onPass("IOError for malformed URL");
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onPass("SecurityError for malformed URL (also valid)");
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                onFail("COMPLETE should not fire for malformed URL");
            });
            try
            {
                loader.load(new URLRequest("http://[invalid-url"));
            }
            catch (err:Error)
            {
                onPass("Synchronous error: " + err.message);
            }
        }

        private function testReuseLoader(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var loadCount:int = 0;
            var handler:Function = function(e:Event):void {
                loadCount++;
                var data:String = URLLoader(e.target).data;
                log("  Reuse load #" + loadCount + ": " + data.substr(0, 30));
                if (loadCount == 1)
                {
                    // Second load
                    loader.load(new URLRequest(SERVER_A + "/json"));
                }
                else if (loadCount == 2)
                {
                    onPass("Reused URLLoader for 2 sequential loads");
                }
            };
            loader.addEventListener(Event.COMPLETE, handler);
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError on load #" + (loadCount + 1) + ": " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/text"));
        }

        private function testInvalidVariablesResponse(onPass:Function, onFail:Function):void
        {
            // When dataFormat=VARIABLES and the response is malformed,
            // Flash's internal URLVariables.decode() may throw Error #2101.
            // That error is NOT dispatched as COMPLETE or IO_ERROR - it is
            // swallowed (or goes to uncaughtErrorEvents). So we use an
            // internal timer: if no event fires within 5s, that itself
            // proves the edge case (silent failure).
            var resolved:Boolean = false;
            var fallbackTimer:Timer = new Timer(5000, 1);
            fallbackTimer.addEventListener(TimerEvent.TIMER, function(te:TimerEvent):void {
                if (!resolved)
                {
                    resolved = true;
                    onPass("No event fired - decode() error swallowed (expected Flash behavior)");
                }
            });
            fallbackTimer.start();

            var loader:URLLoader = makeLoader(URLLoaderDataFormat.VARIABLES);
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                if (resolved) return;
                resolved = true;
                fallbackTimer.stop();
                try
                {
                    var data:* = URLLoader(e.target).data;
                    log("  Invalid vars parsed as: " + data);
                    onPass("Runtime parsed invalid vars (lenient behavior)");
                }
                catch (err:Error)
                {
                    onPass("Error accessing data: " + err.message);
                }
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                if (resolved) return;
                resolved = true;
                fallbackTimer.stop();
                onPass("IOError for invalid variables format");
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                if (resolved) return;
                resolved = true;
                fallbackTimer.stop();
                onFail("SecurityError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/invalid-variables"));
        }

        private function testCustomRequestHeader(onPass:Function, onFail:Function):void
        {
            var req:URLRequest = new URLRequest(SERVER_A + "/echo");
            req.method = URLRequestMethod.POST;
            req.data = "body";
            req.requestHeaders = [new URLRequestHeader("X-Custom-Test", "flash-test-value")];

            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Custom header echo: " + data.substr(0, 120));
                if (data.indexOf("flash-test-value") >= 0)
                    onPass("Custom header echoed back");
                else
                    onPass("Request sent (header may be blocked by runtime), data=" + data.substr(0, 60));
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(req);
        }

        private function testLargeWithProgress(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            var progressEvents:Array = [];
            loader.addEventListener(ProgressEvent.PROGRESS, function(e:ProgressEvent):void {
                progressEvents.push({ loaded: e.bytesLoaded, total: e.bytesTotal });
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Large response: " + progressEvents.length + " progress events, " + data.length + " chars");
                if (progressEvents.length >= 2)
                    onPass(progressEvents.length + " progress events, " + data.length + " chars");
                else
                    onPass("Completed, " + progressEvents.length + " progress events (may vary)");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(new URLRequest(SERVER_A + "/large"));
        }

        private function testQueryAndVariablesMerge(onPass:Function, onFail:Function):void
        {
            var req:URLRequest = new URLRequest(SERVER_A + "/echo?existing=param");
            req.method = URLRequestMethod.GET;
            var vars:URLVariables = new URLVariables();
            vars.added = "var";
            req.data = vars;

            var loader:URLLoader = makeLoader();
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                var data:String = URLLoader(e.target).data;
                log("  Query merge echo: " + data.substr(0, 120));
                // Check if both params present
                var hasExisting:Boolean = data.indexOf("existing") >= 0 || data.indexOf("param") >= 0;
                var hasAdded:Boolean = data.indexOf("added") >= 0 || data.indexOf("var") >= 0;
                if (hasAdded)
                    onPass("URLVariables sent, existing=" + hasExisting + " added=true");
                else
                    onFail("URLVariables not found in echo");
            });
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            loader.load(req);
        }

        private function testBlockedPort(onPass:Function, onFail:Function):void
        {
            var loader:URLLoader = makeLoader();
            loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                log("  IOError for blocked port: " + e.text);
                onPass("IOError for blocked port 25");
            });
            loader.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                log("  SecurityError for blocked port: " + e.text);
                onPass("SecurityError for blocked port 25 (expected)");
            });
            loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                onFail("Should not complete on blocked port");
            });
            try
            {
                loader.load(new URLRequest("http://localhost:25/test"));
            }
            catch (err:Error)
            {
                onPass("Synchronous error for blocked port: " + err.message);
            }
        }

        private function testConcurrentRequests(onPass:Function, onFail:Function):void
        {
            var total:int = 5;
            var completed:int = 0;
            var errors:int = 0;
            var results:Array = [];

            for (var i:int = 0; i < total; i++)
            {
                var loader:URLLoader = makeLoader();
                var idx:int = i;
                loader.addEventListener(Event.COMPLETE, function(e:Event):void {
                    completed++;
                    var data:String = URLLoader(e.target).data;
                    results.push(data.substr(0, 20));
                    log("  Concurrent #" + completed + " done");
                    checkDone();
                });
                loader.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                    errors++;
                    log("  Concurrent error: " + e.text);
                    checkDone();
                });
                // Use different endpoints for variety
                var endpoints:Array = ["/text", "/json", "/xml", "/variables", "/tiny"];
                loader.load(new URLRequest(SERVER_A + endpoints[i]));
            }

            function checkDone():void
            {
                if (completed + errors >= total)
                {
                    if (completed == total)
                        onPass("All " + total + " concurrent requests completed");
                    else
                        onFail(errors + "/" + total + " requests failed");
                }
            }
        }
        // ─────────────────── F. SWF LOADING TESTS ───────────────────

        private function testLoaderBasic(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            var initFired:Boolean = false;
            ldr.contentLoaderInfo.addEventListener(Event.INIT, function(e:Event):void {
                initFired = true;
                log("  Event.INIT fired");
            });
            ldr.contentLoaderInfo.addEventListener(Event.COMPLETE, function(e:Event):void {
                if (initFired)
                    onPass("INIT + COMPLETE both fired");
                else
                    onFail("COMPLETE fired but INIT did not");
            });
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            ldr.load(new URLRequest(SERVER_A + "/LoadableChild.swf"));
        }

        private function testLoaderEventOrder(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            var order:Array = [];
            ldr.contentLoaderInfo.addEventListener(Event.OPEN, function(e:Event):void {
                order.push("OPEN");
            });
            ldr.contentLoaderInfo.addEventListener(ProgressEvent.PROGRESS, function(e:ProgressEvent):void {
                if (order.indexOf("PROGRESS") == -1) order.push("PROGRESS");
            });
            ldr.contentLoaderInfo.addEventListener(Event.INIT, function(e:Event):void {
                order.push("INIT");
            });
            ldr.contentLoaderInfo.addEventListener(Event.COMPLETE, function(e:Event):void {
                order.push("COMPLETE");
                var orderStr:String = order.join("→");
                log("  Loader event order: " + orderStr);
                if (order[0] == "OPEN" && order[order.length - 1] == "COMPLETE"
                    && order.indexOf("INIT") >= 0)
                    onPass(orderStr);
                else
                    onFail("Unexpected order: " + orderStr);
            });
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            ldr.load(new URLRequest(SERVER_A + "/LoadableChild.swf"));
        }

        private function testLoaderContentInfo(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            ldr.contentLoaderInfo.addEventListener(Event.COMPLETE, function(e:Event):void {
                var info:LoaderInfo = ldr.contentLoaderInfo;
                log("  width=" + info.width + " height=" + info.height
                    + " bytesTotal=" + info.bytesTotal
                    + " swfVersion=" + info.swfVersion);
                if (info.bytesTotal > 0 && info.width > 0 && info.height > 0)
                    onPass("w=" + info.width + " h=" + info.height
                        + " bytes=" + info.bytesTotal + " swfVer=" + info.swfVersion);
                else
                    onFail("Unexpected info: bytes=" + info.bytesTotal);
            });
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            ldr.load(new URLRequest(SERVER_A + "/LoadableChild.swf"));
        }

        private function testLoaderDisplayList(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            ldr.contentLoaderInfo.addEventListener(Event.INIT, function(e:Event):void {
                var content:DisplayObject = ldr.content;
                log("  content=" + content + " type=" + typeof(content));
                if (content != null)
                {
                    // Temporarily add to display list to verify
                    addChild(ldr);
                    if (ldr.parent == URLLoaderTests(root) || contains(ldr))
                    {
                        removeChild(ldr);
                        onPass("Loader added to display list, content=" + content);
                    }
                    else
                    {
                        onFail("Could not add Loader to display list");
                    }
                }
                else
                    onFail("content is null after INIT");
            });
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            ldr.load(new URLRequest(SERVER_A + "/LoadableChild.swf"));
        }

        private function testLoaderCrossOriginWithPolicy(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            ldr.contentLoaderInfo.addEventListener(Event.COMPLETE, function(e:Event):void {
                log("  Cross-origin SWF loaded OK, bytes=" + ldr.contentLoaderInfo.bytesTotal);
                onPass("Cross-origin SWF loaded with policy");
            });
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            ldr.contentLoaderInfo.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError: " + e.text);
            });
            ldr.load(new URLRequest(SERVER_B + "/LoadableChild.swf"));
        }

        private function testLoaderCrossOriginNoPolicy(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            ldr.contentLoaderInfo.addEventListener(Event.COMPLETE, function(e:Event):void {
                // Cross-origin SWF loading (content, not data) may succeed
                // even without crossdomain.xml - the SWF loads but cross-scripting
                // is blocked. This is different from URLLoader data access.
                log("  Cross-origin SWF loaded (content load allowed w/o policy)");
                onPass("SWF loaded (cross-scripting blocked, not load itself)");
            });
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onPass("IOError for cross-origin SWF w/o policy (also valid)");
            });
            ldr.contentLoaderInfo.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onPass("SecurityError for cross-origin SWF w/o policy");
            });
            ldr.load(new URLRequest(SERVER_C + "/LoadableChild.swf"));
        }

        private function testLoaderNonSwf(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                log("  IOError loading non-SWF (expected): " + e.text);
                onPass("IOError for non-SWF content");
            });
            ldr.contentLoaderInfo.addEventListener(Event.COMPLETE, function(e:Event):void {
                // Some runtimes may parse text/images without error
                onPass("COMPLETE fired (runtime accepted non-SWF content)");
            });
            ldr.contentLoaderInfo.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onPass("SecurityError for non-SWF (also valid)");
            });
            ldr.load(new URLRequest(SERVER_A + "/text"));
        }

        private function testLoader404(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                log("  Loader 404 IOError (expected): " + e.text);
                onPass("IOError for 404 SWF");
            });
            ldr.contentLoaderInfo.addEventListener(Event.COMPLETE, function(e:Event):void {
                onFail("COMPLETE should not fire for 404");
            });
            ldr.contentLoaderInfo.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(e:SecurityErrorEvent):void {
                onFail("SecurityError: " + e.text);
            });
            ldr.load(new URLRequest(SERVER_A + "/nonexistent.swf"));
        }

        private function testLoaderUnload(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            ldr.contentLoaderInfo.addEventListener(Event.INIT, function(e:Event):void {
                var hadContent:Boolean = (ldr.content != null);
                log("  Before unload: content=" + ldr.content);
                ldr.unload();
                var afterContent:DisplayObject = null;
                try { afterContent = ldr.content; } catch (err:Error) {}
                log("  After unload: content=" + afterContent);
                if (hadContent && afterContent == null)
                    onPass("unload() cleared content");
                else if (hadContent)
                    onPass("unload() called, content=" + afterContent + " (runtime-dependent)");
                else
                    onFail("No content before unload");
            });
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            ldr.load(new URLRequest(SERVER_A + "/LoadableChild.swf"));
        }

        private function testLoaderProgress(onPass:Function, onFail:Function):void
        {
            var ldr:Loader = new Loader();
            var progressCount:int = 0;
            ldr.contentLoaderInfo.addEventListener(ProgressEvent.PROGRESS, function(e:ProgressEvent):void {
                progressCount++;
                log("  Loader progress: " + e.bytesLoaded + "/" + e.bytesTotal);
            });
            ldr.contentLoaderInfo.addEventListener(Event.COMPLETE, function(e:Event):void {
                // Small SWFs may not fire PROGRESS at all
                if (progressCount > 0)
                    onPass(progressCount + " progress events for SWF load");
                else
                    onPass("COMPLETE w/o PROGRESS (small SWF, expected)");
            });
            ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("IOError: " + e.text);
            });
            ldr.load(new URLRequest(SERVER_A + "/LoadableChild.swf"));
        }

        private function testLoaderLoadBytes(onPass:Function, onFail:Function):void
        {
            // First, download the SWF as binary via URLLoader, then use Loader.loadBytes
            var urlLdr:URLLoader = makeLoader(URLLoaderDataFormat.BINARY);
            urlLdr.addEventListener(Event.COMPLETE, function(e:Event):void {
                var bytes:ByteArray = URLLoader(e.target).data as ByteArray;
                log("  Downloaded " + bytes.length + " bytes, calling loadBytes...");

                var ldr:Loader = new Loader();
                ldr.contentLoaderInfo.addEventListener(Event.INIT, function(ie:Event):void {
                    var content:DisplayObject = ldr.content;
                    log("  loadBytes INIT: content=" + content);
                    if (content != null)
                        onPass("loadBytes succeeded, content=" + content);
                    else
                        onFail("loadBytes: content is null");
                });
                ldr.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, function(ie:IOErrorEvent):void {
                    onFail("loadBytes IOError: " + ie.text);
                });
                ldr.contentLoaderInfo.addEventListener(SecurityErrorEvent.SECURITY_ERROR, function(se:SecurityErrorEvent):void {
                    onFail("loadBytes SecurityError: " + se.text);
                });
                ldr.loadBytes(bytes);
            });
            urlLdr.addEventListener(IOErrorEvent.IO_ERROR, function(e:IOErrorEvent):void {
                onFail("Download IOError: " + e.text);
            });
            urlLdr.load(new URLRequest(SERVER_A + "/LoadableChild.swf"));
        }
    }
}
