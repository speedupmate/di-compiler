<?php
namespace Magento\Framework\App;

interface ActionInterface
{
    public function execute(): \Magento\Framework\Controller\ResultInterface;
}
