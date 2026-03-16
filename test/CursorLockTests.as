package
{
    import flash.display.Sprite;
    import flash.display.Shape;
    import flash.display.StageDisplayState;
    import flash.text.TextField;
    import flash.text.TextFieldAutoSize;
    import flash.text.TextFormat;
    import flash.events.Event;
    import flash.events.MouseEvent;
    import flash.events.FullScreenEvent;
    import flash.ui.Mouse;

    [SWF(width="800", height="600", backgroundColor="#1e1e2e", frameRate="30")]
    public class CursorLockTests extends Sprite
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

        private var logField:TextField;
        private var logContent:String = "";
        private var buttonY:int = 50;

        // Crosshair for tracking mouse position
        private var crosshair:Shape;
        private var posLabel:TextField;
        private var deltaLabel:TextField;
        private var cursorHidden:Boolean = false;
        private var mouseLocked:Boolean = false;

        // Draggable sprite
        private var draggableCircle:Sprite;

        public function CursorLockTests()
        {
            addEventListener(Event.ADDED_TO_STAGE, onAddedToStage);
        }

        private function onAddedToStage(e:Event):void
        {
            removeEventListener(Event.ADDED_TO_STAGE, onAddedToStage);
            drawUI();

            // Mouse tracking
            stage.addEventListener(MouseEvent.MOUSE_MOVE, onMouseMove);
            stage.addEventListener(MouseEvent.MOUSE_DOWN, onStageMouseDown);
            stage.addEventListener(MouseEvent.MOUSE_UP, onStageMouseUp);
            stage.addEventListener(MouseEvent.CLICK, onStageClick);

            // Fullscreen events (needed for mouse lock)
            stage.addEventListener(FullScreenEvent.FULL_SCREEN, onFullScreenChange);
            stage.addEventListener(Event.MOUSE_LEAVE, onMouseLeave);
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
            title.text = "Mouse / Cursor Lock Test Suite";
            title.autoSize = TextFieldAutoSize.LEFT;
            title.selectable = false;
            title.x = 10;
            title.y = 10;
            addChild(title);

            // Buttons column
            buttonY = 50;

            createButton("Hide Cursor", function(e:MouseEvent):void {
                Mouse.hide();
                cursorHidden = true;
                appendLog("Mouse.hide() called", COLOR_INFO);
            });

            createButton("Show Cursor", function(e:MouseEvent):void {
                Mouse.show();
                cursorHidden = false;
                appendLog("Mouse.show() called", COLOR_INFO);
            });

            createButton("Enter Fullscreen (for lock)", function(e:MouseEvent):void {
                appendLog("Entering fullscreen mode...", COLOR_INFO);
                try {
                    stage.displayState = StageDisplayState.FULL_SCREEN;
                } catch (err:Error) {
                    appendLog("Error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Enter Fullscreen Interactive", function(e:MouseEvent):void {
                appendLog("Entering fullscreen interactive mode...", COLOR_INFO);
                try {
                    stage.displayState = StageDisplayState.FULL_SCREEN_INTERACTIVE;
                } catch (err:Error) {
                    appendLog("Error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Enable Mouse Lock", function(e:MouseEvent):void {
                appendLog("Attempting to enable mouse lock...", COLOR_INFO);
                try {
                    stage.mouseLock = true;
                    mouseLocked = true;
                    appendLog("stage.mouseLock = true (active in fullscreen)", COLOR_SUCCESS);
                } catch (err:Error) {
                    appendLog("Mouse lock error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Disable Mouse Lock", function(e:MouseEvent):void {
                try {
                    stage.mouseLock = false;
                    mouseLocked = false;
                    appendLog("stage.mouseLock = false", COLOR_INFO);
                } catch (err:Error) {
                    appendLog("Error: " + err.message, COLOR_ERROR);
                }
            });

            createButton("Exit Fullscreen", function(e:MouseEvent):void {
                appendLog("Exiting fullscreen...", COLOR_INFO);
                stage.displayState = StageDisplayState.NORMAL;
            });

            createButton("Check Cursor State", function(e:MouseEvent):void {
                appendLog("Cursor hidden: " + cursorHidden, COLOR_INFO);
                appendLog("Mouse locked: " + mouseLocked, COLOR_INFO);
                appendLog("Display state: " + stage.displayState, COLOR_INFO);
                try {
                    appendLog("stage.mouseLock: " + stage.mouseLock, COLOR_INFO);
                } catch (err:Error) {
                    appendLog("stage.mouseLock not available: " + err.message, COLOR_DIM);
                }
            });

            createButton("Clear Log", function(e:MouseEvent):void {
                logContent = "";
                logField.text = "";
            });

            // Draggable circle demo
            draggableCircle = new Sprite();
            draggableCircle.graphics.beginFill(0x89b4fa);
            draggableCircle.graphics.drawCircle(0, 0, 25);
            draggableCircle.graphics.endFill();
            draggableCircle.x = 500;
            draggableCircle.y = 200;
            draggableCircle.buttonMode = true;
            draggableCircle.addEventListener(MouseEvent.MOUSE_DOWN, onCircleDown);
            addChild(draggableCircle);

            var circleLabel:TextField = new TextField();
            circleLabel.defaultTextFormat = new TextFormat("_sans", 9, COLOR_DIM);
            circleLabel.text = "Drag me!";
            circleLabel.autoSize = TextFieldAutoSize.CENTER;
            circleLabel.selectable = false;
            circleLabel.mouseEnabled = false;
            circleLabel.x = draggableCircle.x - 25;
            circleLabel.y = draggableCircle.y - 35;
            addChild(circleLabel);

            // Position label
            posLabel = new TextField();
            posLabel.defaultTextFormat = new TextFormat("_typewriter", 11, COLOR_INFO);
            posLabel.autoSize = TextFieldAutoSize.LEFT;
            posLabel.selectable = false;
            posLabel.x = 310;
            posLabel.y = 52;
            addChild(posLabel);

            // Delta label (for mouse lock movement deltas)
            deltaLabel = new TextField();
            deltaLabel.defaultTextFormat = new TextFormat("_typewriter", 11, COLOR_SUCCESS);
            deltaLabel.autoSize = TextFieldAutoSize.LEFT;
            deltaLabel.selectable = false;
            deltaLabel.x = 310;
            deltaLabel.y = 70;
            addChild(deltaLabel);

            // Crosshair
            crosshair = new Shape();
            crosshair.graphics.lineStyle(1, 0xff6600, 0.8);
            crosshair.graphics.moveTo(-15, 0);
            crosshair.graphics.lineTo(15, 0);
            crosshair.graphics.moveTo(0, -15);
            crosshair.graphics.lineTo(0, 15);
            crosshair.visible = false;
            addChild(crosshair);

            // Log area
            var logBg:Shape = new Shape();
            logBg.graphics.beginFill(COLOR_LOG_BG);
            logBg.graphics.drawRect(300, 90, 490, 500);
            logBg.graphics.endFill();
            addChild(logBg);

            var logFmt:TextFormat = new TextFormat("_typewriter", 11, COLOR_TEXT);
            logField = new TextField();
            logField.defaultTextFormat = logFmt;
            logField.x = 305;
            logField.y = 95;
            logField.width = 480;
            logField.height = 490;
            logField.multiline = true;
            logField.wordWrap = true;
            logField.selectable = true;
            addChild(logField);

            appendLog("Cursor Lock Test Suite ready.", COLOR_SUCCESS);
            appendLog("Move mouse to see tracking. Try buttons.", COLOR_DIM);
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

        private function onMouseMove(e:MouseEvent):void
        {
            crosshair.x = e.stageX;
            crosshair.y = e.stageY;
            crosshair.visible = true;

            posLabel.text = "Mouse: (" + int(e.stageX) + ", " + int(e.stageY) +
                ") local: (" + int(e.localX) + ", " + int(e.localY) + ")";

            // Show movement deltas when available (mouse lock mode)
            try {
                var dx:Number = e["movementX"];
                var dy:Number = e["movementY"];
                if (!isNaN(dx) && !isNaN(dy)) {
                    deltaLabel.text = "Delta: (" + dx + ", " + dy + ")";
                } else {
                    deltaLabel.text = "";
                }
            } catch (err:Error) {
                deltaLabel.text = "";
            }
        }

        private function onStageMouseDown(e:MouseEvent):void
        {
            appendLog("MOUSE_DOWN at (" + int(e.stageX) + "," + int(e.stageY) +
                ") button=" + formatButton(e) +
                " shift=" + e.shiftKey + " ctrl=" + e.ctrlKey + " alt=" + e.altKey, COLOR_DIM);
        }

        private function onStageMouseUp(e:MouseEvent):void
        {
            // Only log if not in button area
            if (e.stageX > 300) {
                appendLog("MOUSE_UP at (" + int(e.stageX) + "," + int(e.stageY) + ")", COLOR_DIM);
            }
        }

        private function onStageClick(e:MouseEvent):void
        {
            if (e.stageX > 300) {
                appendLog("CLICK at (" + int(e.stageX) + "," + int(e.stageY) + ")", COLOR_INFO);
            }
        }

        private function onMouseLeave(e:Event):void
        {
            crosshair.visible = false;
            appendLog("MOUSE_LEAVE: cursor left the stage", COLOR_DIM);
        }

        private function onCircleDown(e:MouseEvent):void
        {
            e.stopPropagation();
            appendLog("Drag started on circle", COLOR_INFO);
            draggableCircle.startDrag();
            stage.addEventListener(MouseEvent.MOUSE_UP, onCircleUp);
        }

        private function onCircleUp(e:MouseEvent):void
        {
            draggableCircle.stopDrag();
            stage.removeEventListener(MouseEvent.MOUSE_UP, onCircleUp);
            appendLog("Drag ended. Circle at (" + int(draggableCircle.x) + "," + int(draggableCircle.y) + ")", COLOR_INFO);
        }

        private function onFullScreenChange(e:FullScreenEvent):void
        {
            if (e.fullScreen) {
                appendLog("Entered fullscreen mode", COLOR_SUCCESS);
            } else {
                appendLog("Exited fullscreen mode", COLOR_INFO);
                if (mouseLocked) {
                    mouseLocked = false;
                    appendLog("Mouse lock automatically disabled on exit", COLOR_DIM);
                }
            }
        }

        private function formatButton(e:MouseEvent):String
        {
            // AS3 MouseEvent doesn't have a direct "button" property on MOUSE_DOWN
            // but we can determine from the event type
            return "left";
        }

        private function appendLog(msg:String, color:uint = 0xcdd6f4):void
        {
            trace("[CursorLockTest] " + msg);
            var fmt:TextFormat = new TextFormat("_typewriter", 11, color);
            var startIdx:int = logField.text.length;
            logField.appendText(msg + "\n");
            logField.setTextFormat(fmt, startIdx, logField.text.length);
            logField.scrollV = logField.maxScrollV;
        }
    }
}
