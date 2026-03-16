package
{
    import flash.display.Sprite;
    import flash.display.Shape;
    import flash.display.Stage;
    import flash.display.StageDisplayState;
    import flash.display.StageScaleMode;
    import flash.display.StageAlign;
    import flash.text.TextField;
    import flash.text.TextFieldAutoSize;
    import flash.text.TextFormat;
    import flash.events.Event;
    import flash.events.MouseEvent;
    import flash.events.FullScreenEvent;
    import flash.events.KeyboardEvent;
    import flash.events.TimerEvent;
    import flash.system.Capabilities;
    import flash.geom.Rectangle;
    import flash.utils.Timer;

    [SWF(width="800", height="600", backgroundColor="#1e1e2e", frameRate="30")]
    public class FullscreenTests extends Sprite
    {
        // Colors
        private static const COLOR_BG:uint         = 0x1e1e2e;
        private static const COLOR_HEADER_BG:uint   = 0x181825;
        private static const COLOR_BUTTON:uint      = 0x313244;
        private static const COLOR_BUTTON_HOVER:uint = 0x45475a;
        private static const COLOR_TEXT:uint         = 0xcdd6f4;
        private static const COLOR_DIM:uint          = 0x6c7086;
        private static const COLOR_LOG_BG:uint       = 0x11111b;
        private static const COLOR_SUCCESS:uint      = 0xa6e3a1;
        private static const COLOR_ERROR:uint        = 0xf38ba8;
        private static const COLOR_INFO:uint         = 0x89b4fa;
        private static const COLOR_WARN:uint         = 0xf9e2af;

        private var logField:TextField;
        private var logContent:String = "";
        private var buttonY:int = 50;
        private var statusField:TextField;

        // Status polling timer
        private var statusTimer:Timer;

        public function FullscreenTests()
        {
            addEventListener(Event.ADDED_TO_STAGE, onAddedToStage);
        }

        private function onAddedToStage(e:Event):void
        {
            removeEventListener(Event.ADDED_TO_STAGE, onAddedToStage);

            // Listen for fullscreen events
            stage.addEventListener(FullScreenEvent.FULL_SCREEN, onFullScreenEvent);
            stage.addEventListener(FullScreenEvent.FULL_SCREEN_INTERACTIVE_ACCEPTED, onFullScreenInteractive);
            stage.addEventListener(Event.RESIZE, onStageResize);
            stage.addEventListener(KeyboardEvent.KEY_DOWN, onKeyDown);

            drawUI();

            // Poll status periodically
            statusTimer = new Timer(500);
            statusTimer.addEventListener(TimerEvent.TIMER, updateStatus);
            statusTimer.start();
        }

        private function drawUI():void
        {
            // Background
            var bg:Shape = new Shape();
            bg.graphics.beginFill(COLOR_BG);
            bg.graphics.drawRect(0, 0, 800, 600);
            bg.graphics.endFill();
            addChild(bg);

            // Header
            var header:Shape = new Shape();
            header.graphics.beginFill(COLOR_HEADER_BG);
            header.graphics.drawRect(0, 0, 800, 40);
            header.graphics.endFill();
            addChild(header);

            var titleFmt:TextFormat = new TextFormat("_sans", 14, COLOR_TEXT, true);
            var title:TextField = new TextField();
            title.defaultTextFormat = titleFmt;
            title.text = "Fullscreen Mode Test Suite";
            title.autoSize = TextFieldAutoSize.LEFT;
            title.selectable = false;
            title.x = 10;
            title.y = 10;
            addChild(title);

            // Buttons
            buttonY = 50;

            createButton("Enter FULL_SCREEN", function(e:MouseEvent):void {
                appendLog("Setting displayState = FULL_SCREEN...", COLOR_INFO);
                try {
                    stage.displayState = StageDisplayState.FULL_SCREEN;
                    appendLog("displayState set successfully", COLOR_SUCCESS);
                } catch (err:Error) {
                    appendLog("Error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Enter FULL_SCREEN_INTERACTIVE", function(e:MouseEvent):void {
                appendLog("Setting displayState = FULL_SCREEN_INTERACTIVE...", COLOR_INFO);
                try {
                    stage.displayState = StageDisplayState.FULL_SCREEN_INTERACTIVE;
                    appendLog("displayState set successfully", COLOR_SUCCESS);
                } catch (err:Error) {
                    appendLog("Error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Exit Fullscreen (NORMAL)", function(e:MouseEvent):void {
                appendLog("Setting displayState = NORMAL...", COLOR_INFO);
                try {
                    stage.displayState = StageDisplayState.NORMAL;
                } catch (err:Error) {
                    appendLog("Error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Set fullScreenSourceRect", function(e:MouseEvent):void {
                appendLog("Setting fullScreenSourceRect(0, 0, 400, 300)...", COLOR_INFO);
                try {
                    stage.fullScreenSourceRect = new Rectangle(0, 0, 400, 300);
                    appendLog("fullScreenSourceRect set. Enter fullscreen to see zoom.", COLOR_SUCCESS);
                } catch (err:Error) {
                    appendLog("Error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Clear fullScreenSourceRect", function(e:MouseEvent):void {
                appendLog("Clearing fullScreenSourceRect...", COLOR_INFO);
                try {
                    stage.fullScreenSourceRect = null;
                    appendLog("fullScreenSourceRect cleared", COLOR_SUCCESS);
                } catch (err:Error) {
                    appendLog("Error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Set scaleMode: NO_SCALE", function(e:MouseEvent):void {
                stage.scaleMode = StageScaleMode.NO_SCALE;
                stage.align = StageAlign.TOP_LEFT;
                appendLog("scaleMode = NO_SCALE, align = TOP_LEFT", COLOR_INFO);
            });

            createButton("Set scaleMode: SHOW_ALL", function(e:MouseEvent):void {
                stage.scaleMode = StageScaleMode.SHOW_ALL;
                appendLog("scaleMode = SHOW_ALL", COLOR_INFO);
            });

            createButton("Set scaleMode: EXACT_FIT", function(e:MouseEvent):void {
                stage.scaleMode = StageScaleMode.EXACT_FIT;
                appendLog("scaleMode = EXACT_FIT", COLOR_INFO);
            });

            createButton("Check Status", function(e:MouseEvent):void {
                logCurrentStatus();
            });

            createButton("Get Screen Info", function(e:MouseEvent):void {
                appendLog("Screen: " + Capabilities.screenResolutionX + "x" +
                    Capabilities.screenResolutionY, COLOR_INFO);
                appendLog("Stage: " + stage.stageWidth + "x" + stage.stageHeight, COLOR_INFO);
                appendLog("DPI: " + Capabilities.screenDPI, COLOR_INFO);
                appendLog("Player: " + Capabilities.version, COLOR_INFO);
            });

            createButton("Clear Log", function(e:MouseEvent):void {
                logContent = "";
                logField.text = "";
            });

            // Status display
            var statusBg:Shape = new Shape();
            statusBg.graphics.beginFill(COLOR_HEADER_BG);
            statusBg.graphics.drawRect(300, 50, 490, 50);
            statusBg.graphics.endFill();
            addChild(statusBg);

            statusField = new TextField();
            statusField.defaultTextFormat = new TextFormat("_typewriter", 11, COLOR_INFO);
            statusField.x = 305;
            statusField.y = 52;
            statusField.width = 480;
            statusField.height = 45;
            statusField.multiline = true;
            statusField.selectable = false;
            addChild(statusField);

            // Log area
            var logBg:Shape = new Shape();
            logBg.graphics.beginFill(COLOR_LOG_BG);
            logBg.graphics.drawRect(300, 105, 490, 485);
            logBg.graphics.endFill();
            addChild(logBg);

            var logFmt:TextFormat = new TextFormat("_typewriter", 11, COLOR_TEXT);
            logField = new TextField();
            logField.defaultTextFormat = logFmt;
            logField.x = 305;
            logField.y = 110;
            logField.width = 480;
            logField.height = 475;
            logField.multiline = true;
            logField.wordWrap = true;
            logField.selectable = true;
            addChild(logField);

            appendLog("Fullscreen Test Suite ready.", COLOR_SUCCESS);
            appendLog("Press ESC to exit fullscreen.", COLOR_DIM);
            appendLog("allowFullScreen must be 'true' in HTML embed.", COLOR_DIM);
        }

        private function createButton(label:String, handler:Function):void
        {
            var btn:Sprite = new Sprite();
            btn.graphics.beginFill(COLOR_BUTTON);
            btn.graphics.drawRoundRect(0, 0, 280, 32, 6, 6);
            btn.graphics.endFill();
            btn.x = 10;
            btn.y = buttonY;
            btn.buttonMode = true;
            btn.useHandCursor = true;

            var fmt:TextFormat = new TextFormat("_sans", 11, COLOR_TEXT);
            var tf:TextField = new TextField();
            tf.defaultTextFormat = fmt;
            tf.text = label;
            tf.autoSize = TextFieldAutoSize.LEFT;
            tf.selectable = false;
            tf.mouseEnabled = false;
            tf.x = 10;
            tf.y = 7;
            btn.addChild(tf);

            btn.addEventListener(MouseEvent.CLICK, handler);
            btn.addEventListener(MouseEvent.ROLL_OVER, function(e:MouseEvent):void {
                btn.graphics.clear();
                btn.graphics.beginFill(COLOR_BUTTON_HOVER);
                btn.graphics.drawRoundRect(0, 0, 280, 32, 6, 6);
                btn.graphics.endFill();
            });
            btn.addEventListener(MouseEvent.ROLL_OUT, function(e:MouseEvent):void {
                btn.graphics.clear();
                btn.graphics.beginFill(COLOR_BUTTON);
                btn.graphics.drawRoundRect(0, 0, 280, 32, 6, 6);
                btn.graphics.endFill();
            });

            addChild(btn);
            buttonY += 38;
        }

        private function updateStatus(e:TimerEvent = null):void
        {
            var state:String = "unknown";
            try { state = stage.displayState; } catch (err:Error) {}

            var w:int = stage.stageWidth;
            var h:int = stage.stageHeight;

            statusField.text = "displayState: " + state + "  |  Stage: " + w + "x" + h +
                "\nscaleMode: " + stage.scaleMode + "  |  align: " + stage.align;
        }

        private function logCurrentStatus():void
        {
            appendLog("--- Current Status ---", COLOR_WARN);
            try {
                appendLog("displayState: " + stage.displayState, COLOR_INFO);
            } catch (err:Error) {
                appendLog("displayState: error - " + err.message, COLOR_ERROR);
            }
            appendLog("stageWidth: " + stage.stageWidth, COLOR_INFO);
            appendLog("stageHeight: " + stage.stageHeight, COLOR_INFO);
            appendLog("scaleMode: " + stage.scaleMode, COLOR_INFO);
            appendLog("align: " + stage.align, COLOR_INFO);
            try {
                var fsr:Rectangle = stage.fullScreenSourceRect;
                if (fsr) {
                    appendLog("fullScreenSourceRect: " + fsr.toString(), COLOR_INFO);
                } else {
                    appendLog("fullScreenSourceRect: null", COLOR_DIM);
                }
            } catch (err:Error) {
                appendLog("fullScreenSourceRect: error - " + err.message, COLOR_ERROR);
            }
            appendLog("--- End Status ---", COLOR_WARN);
        }

        // Event handlers
        private function onFullScreenEvent(e:FullScreenEvent):void
        {
            if (e.fullScreen) {
                appendLog("EVENT: FullScreenEvent - ENTERED fullscreen", COLOR_SUCCESS);
                appendLog("  interactive: " + e.interactive, COLOR_DIM);
            } else {
                appendLog("EVENT: FullScreenEvent - EXITED fullscreen", COLOR_INFO);
            }
        }

        private function onFullScreenInteractive(e:FullScreenEvent):void
        {
            appendLog("EVENT: FULL_SCREEN_INTERACTIVE_ACCEPTED", COLOR_SUCCESS);
        }

        private function onStageResize(e:Event):void
        {
            appendLog("EVENT: Stage RESIZE - " + stage.stageWidth + "x" + stage.stageHeight, COLOR_DIM);
        }

        private function onKeyDown(e:KeyboardEvent):void
        {
            if (e.keyCode == 27) { // ESC
                appendLog("ESC pressed - exiting fullscreen", COLOR_INFO);
            }
        }

        private function appendLog(msg:String, color:uint = 0xcdd6f4):void
        {
            trace("[FullscreenTest] " + msg);
            var fmt:TextFormat = new TextFormat("_typewriter", 11, color);
            var startIdx:int = logField.text.length;
            logField.appendText(msg + "\n");
            logField.setTextFormat(fmt, startIdx, logField.text.length);
            logField.scrollV = logField.maxScrollV;
        }
    }
}
